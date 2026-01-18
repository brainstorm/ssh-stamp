// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use sunset::sshwire::{SSHDecode, SSHSource, WireError};

use crate::{
    OtaHeader,
    target::{self, OtaWriter},
    tlv,
};

use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};

/// UpdateProcessorState for OTA update processing
///
/// This enum defines the various states of the OTA update processing state machine and will control the flow of the update process.
#[derive(Debug)]
enum UpdateProcessorState {
    /// ReadingParameters state, OTA has started and the processor is obtaining metadata values until the firmware blob is reached
    ReadingParameters {
        tlv_holder: [u8; tlv::MAX_TLV_SIZE as usize],
        current_len: usize,
    },
    /// Downloading state, receiving firmware data, computing hash on the fly and writing to flash
    Downloading {
        total_received_size: u32,
        writer: OtaWriter,
    },
    /// Like idle, but after successful verification, ready to reboot and apply the update
    Finished { writer: OtaWriter },
    /// Error state, an error occurred during the OTA process
    Error(OtaError),
}

impl Default for UpdateProcessorState {
    fn default() -> Self {
        UpdateProcessorState::ReadingParameters {
            tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
            current_len: 0,
        }
    }
}

/// # UpdateProcessor for handling OTA update processing
///
/// This struct manages the state and processing of OTA updates received via SFTP. It will handle reading metadata, writing data, verifying, and applying updates.
///
/// It uses an internal state machine defined by [[UpdateProcessorState]] to track the progress of the update process.
///
/// It will also handle incoming data chunks and process them accordingly.
pub(crate) struct UpdateProcessor {
    state: UpdateProcessorState,
    /// Hasher computing the checksum of the downloaded firmware on the fly
    hasher: Sha256,
    header: OtaHeader,
}

impl UpdateProcessor {
    pub fn new() -> Self {
        Self {
            state: UpdateProcessorState::default(),
            hasher: Sha256::new(),
            header: OtaHeader {
                ota_type: None,
                firmware_blob_size: None,
                sha256_checksum: None,
            },
        }
    }

    /// Main processing function for handling incoming data chunks
    ///
    /// It processes data based on the current state of the update processor [[UpdateProcessorState]]. To first, read most metadata parameters, after that, write the data to the appropriate location. as it is received.
    ///
    /// It will try to consume as much data as possible from the provided buffer and return the number of bytes used.
    pub async fn process_data_chunk(&mut self, _offset: u64, data: &[u8]) -> Result<(), OtaError> {
        debug!(
            "UpdateProcessor: Processing data chunk at offset {}, length {} in state {:?}",
            _offset,
            data.len(),
            self.state
        );
        let mut source = tlv::TlvsSource::new(&data);
        while source.remaining() > 0 {
            debug!("processor state : {:?}", self.state);

            match self.state {
                UpdateProcessorState::ReadingParameters {
                    mut tlv_holder,
                    mut current_len,
                } => {
                    match source.try_taking_bytes_for_tlv(&mut tlv_holder, &mut current_len) {
                        Err(WireError::RanOut) => {
                            // Not enough data to complete TLV, wait for more data
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder,
                                current_len,
                            };
                            return Ok(());
                        }
                        Err(e) => {
                            error!("Error processing TLV: {:?}", e);
                            return Err(OtaError::InternalError);
                        }
                        Ok(_) => {
                            // Successfully read a TLV, continue processing
                        }
                    };

                    // At this point there should be a complete TLV to be decoded
                    debug!(
                        "Decoding TLV from tlv_holder: {:?},  current_len: {}",
                        &tlv_holder, &current_len
                    );
                    let mut singular_source = tlv::TlvsSource::new(&tlv_holder[..current_len]);

                    match tlv::Tlv::dec(&mut singular_source) {
                        Ok(tlv) => match tlv {
                            tlv::Tlv::OtaType { ota_type } => {
                                // TODO: If the received ota_type does not match tlv::OTA_TYPE_VALUE_SSH_STAMP go to error state.
                                if ota_type != tlv::OTA_TYPE_VALUE_SSH_STAMP {
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Err(OtaError::IllegalOperation);
                                }
                                info!("Received Ota type: {:?}", ota_type);
                                self.header.ota_type = Some(ota_type);
                                self.state = UpdateProcessorState::ReadingParameters {
                                    tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                    current_len: 0,
                                };
                            }

                            tlv::Tlv::Sha256Checksum { checksum } => {
                                info!("Received Checksum: {:?}", &checksum);
                                if self.header.ota_type.is_none() {
                                    error!(
                                        "UpdateProcessor: Received SHA256 Checksum TLV before OTA Type TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Err(OtaError::IllegalOperation);
                                }
                                self.header.sha256_checksum = Some(checksum);
                                self.state = UpdateProcessorState::ReadingParameters {
                                    tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                    current_len: 0,
                                };
                            }
                            tlv::Tlv::FirmwareBlob { size } => {
                                info!("Received FirmwareBlob size: {:?}", size);
                                if self.header.ota_type.is_none() {
                                    error!(
                                        "UpdateProcessor: Received SHA256 Checksum TLV before OTA Type TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Err(OtaError::IllegalOperation);
                                }

                                if self.header.sha256_checksum.is_none() {
                                    error!(
                                        "UpdateProcessor: Received FirmwareBlob TLV before SHA256 Checksum TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Err(OtaError::IllegalOperation);
                                }

                                self.header.firmware_blob_size = Some(size);

                                // Transition to Downloading state will be done after this match

                                let update_slot =
                                    target::get_next_app_slot().await.map_err(|e| {
                                        error!(
                                            "UpdateProcessor: Error getting OTA update slot: {:?}",
                                            e
                                        );
                                        OtaError::InternalError
                                    })?;
                                info!("Starting OTA update in {update_slot:?}");
                                self.state = UpdateProcessorState::Downloading {
                                    total_received_size: 0,
                                    writer: OtaWriter::new(update_slot),
                                };
                                info!("Transitioning to Downloading state");
                            }
                        },
                        Err(WireError::UnknownPacket { number }) => {
                            warn!(
                                "UpdateProcessor: Unknown TLV type encountered: {}. Skipping it",
                                number
                            );
                            if self.header.ota_type.is_none() {
                                error!(
                                    "UpdateProcessor: Received unknown TLV type before OTA Type TLV"
                                );
                                self.state =
                                    UpdateProcessorState::Error(OtaError::IllegalOperation);
                                return Err(OtaError::IllegalOperation);
                            }
                            // Skip this TLV and continue
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                current_len: 0,
                            }
                        }
                        Err(WireError::RanOut) => {
                            // Keep current data and wait for more
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder,
                                current_len,
                            };
                            error!("UpdateProcessor: RanOut should not be happening");
                            return Err(OtaError::MoreDataRequired);
                        }
                        Err(e) => {
                            error!("Handle {:?} appropriately", e);
                            return Err(OtaError::InternalError);
                        }
                    }
                }
                UpdateProcessorState::Downloading {
                    mut total_received_size,
                    writer,
                } => {
                    let total_blob_size = match self.header.firmware_blob_size {
                        Some(size) => size,
                        None => {
                            error!(
                                "UpdateProcessor: Firmware blob size not set before downloading"
                            );
                            return Err(OtaError::IllegalOperation);
                        }
                    };
                    debug!("source contains {} bytes", source.remaining());
                    // Once the totality of the blob has been received, the FSM must move to the Finished or Error States
                    if total_received_size >= total_blob_size {
                        error!(
                            "UpdateProcessor: Received more data than expected: received_size = {}, total_blob_size = {}",
                            total_received_size, total_blob_size
                        );
                        return Err(OtaError::IllegalOperation);
                    }

                    let to_take = source
                        .remaining()
                        .min((total_blob_size - total_received_size) as usize);

                    let data_chunk = source.take(to_take).map_err(|e| {
                        error!(
                            "UpdateProcessor: Error taking data chunk of size {}: {:?}",
                            to_take, e
                        );
                        OtaError::InternalError
                    })?;

                    self.hasher.update(data_chunk);

                    debug!(
                        "Writing {} bytes to flash at offset {}",
                        data_chunk.len(),
                        total_received_size
                    );
                    writer.write(total_received_size, data_chunk).await.map_err(|e| {
                        error!(
                            "UpdateProcessor: Error writing data chunk to flash at offset {}: {:?}",
                            total_received_size, e
                        );
                        OtaError::WriteError
                    })?;

                    total_received_size += to_take as u32;

                    if total_received_size >= total_blob_size {
                        let Some(original_hash) = self.header.sha256_checksum else {
                            error!(
                                "UpdateProcessor: No original checksum to verify against after download"
                            );
                            return Err(OtaError::IllegalOperation);
                        };

                        if original_hash
                            != *self
                                .hasher
                                .clone()
                                .finalize()
                                .as_array()
                                .ok_or(OtaError::VerificationFailed)?
                        {
                            error!(
                                "UpdateProcessor: Checksum mismatch after download! Expected: {:x?}`",
                                original_hash
                            );
                            self.state = UpdateProcessorState::Error(OtaError::VerificationFailed);
                            return Ok(());
                        } else {
                            info!("UpdateProcessor: Checksum verified successfully");
                        }

                        info!("All firmware data received, transitioning to Finished state");
                        self.state = UpdateProcessorState::Finished { writer };
                    } else {
                        self.state = UpdateProcessorState::Downloading {
                            total_received_size,
                            writer,
                        };
                    }
                }
                UpdateProcessorState::Finished { writer: _ } => {
                    // Will ignore the data. It will be consumed and the file will be closed eventually
                    // This behaviour will allow any future file footer (e.g. signature?) to be discarded
                    //  without causing problems
                    warn!(
                        "UpdateProcessor: Received data in Finished state, ignoring additional data"
                    );
                    return Ok(());
                }
                UpdateProcessorState::Error(ota_error) => {
                    // Will ignore the data. It will be consumed and the file will be closed eventually
                    warn!(
                        "UpdateProcessor: Received data in Error state: {:?}, ignoring additional data",
                        ota_error
                    );
                    return Ok(());
                }
            };
        }
        Ok(())
    }

    pub async fn finalize(&mut self) -> Result<(), OtaError> {
        let ret_val = match self.state {
            UpdateProcessorState::Finished { mut writer } => {
                info!("Finalizing OTA update process successfully.");

                // Here you would trigger the application of the update, e.g., rebooting into the new firmware
                writer.finalize().await.map_err(|e| {
                    error!("Error finalizing OTA update: {:?}", e);
                    OtaError::InternalError
                })
            }
            UpdateProcessorState::Error(e) => {
                error!("Cannot finalize OTA update due to error state: {:?}", e);
                Err(e)
            }
            _ => {
                error!(
                    "Cannot finalize OTA update, current state is not Finished: {:?}",
                    self.state
                );
                Err(OtaError::IllegalOperation)
            }
        };

        self.reset_ota_state();
        ret_val
    }

    fn reset_ota_state(&mut self) {
        info!("Resetting OTA processor state.");
        self.state = UpdateProcessorState::default();
        self.hasher = Sha256::new();
        self.header = OtaHeader {
            ota_type: None,
            firmware_blob_size: None,
            sha256_checksum: None,
        };
    }

    // Add other parameters, such as verify, apply, check signature, etc.
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// OtaError for OTA update processing errors
pub(crate) enum OtaError {
    /// Needs more data to proceed
    MoreDataRequired,
    /// Internal error
    InternalError,
    /// An operation was illegal in the current state
    IllegalOperation,
    /// Error writing data to flash memory
    WriteError,
    /// Verification of the downloaded firmware failed
    VerificationFailed,
}

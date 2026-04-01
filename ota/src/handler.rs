// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use sunset::sshwire::{SSHDecode, SSHSource, WireError};

use crate::{OtaHeader, otatraits::OtaActions, tlv};

use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};

/// `UpdateProcessorState` for OTA update processing
///
/// This enum defines the various states of the OTA update processing state machine and will control the flow of the update process.
#[derive(Debug)]
enum UpdateProcessorState {
    /// `ReadingParameters` state, OTA has started and the processor is obtaining metadata values until the firmware blob is reached
    ReadingParameters {
        // tlv_holder: [u8; tlv::MAX_TLV_SIZE as usize],
        // current_len: usize,
    },
    /// Downloading state, receiving firmware data, computing hash on the fly and writing to flash
    Downloading { total_received_size: u32 },
    /// Like idle, but after successful verification, ready to reboot and apply the update
    Finished {},
    /// Error state, an error occurred during the OTA process
    Error(OtaError),
}

impl Default for UpdateProcessorState {
    fn default() -> Self {
        UpdateProcessorState::ReadingParameters {
                // tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                // current_len: 0,
        }
    }
}

/// # `UpdateProcessor` for handling OTA update processing
///
/// This struct manages the state and processing of OTA updates received via SFTP. It will handle reading metadata, writing data, verifying, and applying updates.
///
/// It uses an internal state machine defined by [[`UpdateProcessorState`]] to track the progress of the update process.
///
/// It will also handle incoming data chunks and process them accordingly.
pub(crate) struct UpdateProcessor<W: OtaActions> {
    state: UpdateProcessorState,
    /// Hasher computing the checksum of the downloaded firmware on the fly
    hasher: Sha256,
    header: OtaHeader,
    ota_writer: W,
    tlv_holder: [u8; tlv::MAX_TLV_SIZE as usize],
    current_len: usize,
}

impl<W: OtaActions> UpdateProcessor<W> {
    /// Creates a new `UpdateProcessor` instance with the given `OtaActions` implementation
    ///
    /// Use this `ota_writer` to perform platform-specific OTA actions
    pub fn new(ota_writer: W) -> Self {
        Self {
            state: UpdateProcessorState::default(),
            hasher: Sha256::new(),
            header: OtaHeader {
                ota_type: None,
                firmware_blob_size: None,
                sha256_checksum: None,
            },
            ota_writer,
            tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
            current_len: 0,
        }
    }

    /// Main processing function for handling incoming data chunks
    ///
    /// It processes data based on the current state of the update processor [[`UpdateProcessorState`]]. To first, read most metadata parameters, after that, write the data to the appropriate location. as it is received.
    ///
    /// It will try to consume as much data as possible from the provided buffer and return the number of bytes used.
    pub async fn process_data(&mut self, offset: u64, data: &[u8]) -> Result<(), OtaError> {
        debug!(
            "UpdateProcessor: Processing data chunk at offset {}, length {} in state {:?}",
            offset,
            data.len(),
            self.state
        );
        let mut source = tlv::TlvsSource::new(data);
        while source.remaining() > 0 {
            debug!("processor state : {:?}", self.state);

            match self.state {
                UpdateProcessorState::ReadingParameters { .. } => {
                    self.process_reading_parameters(&mut source).await?;
                }
                UpdateProcessorState::Downloading {
                    mut total_received_size,
                } => {
                    self.process_downloading(&mut source, &mut total_received_size)
                        .await?;
                }
                UpdateProcessorState::Finished {} => {
                    warn!(
                        "UpdateProcessor: Received data in Finished state, ignoring additional data"
                    );
                    return Ok(());
                }
                UpdateProcessorState::Error(ota_error) => {
                    warn!(
                        "UpdateProcessor: Received data in Error state: {ota_error:?}, ignoring additional data"
                    );
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    async fn process_reading_parameters(
        &mut self,
        source: &mut tlv::TlvsSource<'_>,
    ) -> Result<(), OtaError> {
        match source.try_taking_bytes_for_tlv(&mut self.tlv_holder, &mut self.current_len) {
            Err(WireError::RanOut) => {
                self.state = UpdateProcessorState::ReadingParameters {};
                return Ok(());
            }
            Err(e) => {
                error!("Error processing TLV: {e:?}");
                return Err(OtaError::InternalError);
            }
            Ok(()) => {}
        }

        debug!(
            "Decoding TLV from tlv_holder: {:?},  current_len: {}",
            &self.tlv_holder, &self.current_len
        );
        let mut singular_source = tlv::TlvsSource::new(&self.tlv_holder[..self.current_len]);

        match tlv::Tlv::dec(&mut singular_source) {
            Ok(tlv) => self.handle_tlv(tlv).await?,
            Err(WireError::UnknownPacket { number }) => {
                if self.header.ota_type.is_none() {
                    error!("UpdateProcessor: Received unknown TLV type before OTA Type TLV");
                    self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
                    return Err(OtaError::IllegalOperation);
                }
                error!("UpdateProcessor: Unknown TLV type encountered: {number}");
                return Err(OtaError::UnknownTlvType);
            }
            Err(WireError::RanOut) => {
                self.tlv_holder.fill(0);
                self.current_len = 0;
                error!("UpdateProcessor: RanOut should not be happening");
                return Err(OtaError::MoreDataRequired);
            }
            Err(e) => {
                error!("Handle {e:?} appropriately");
                return Err(OtaError::InternalError);
            }
        }
        Ok(())
    }

    async fn handle_tlv(&mut self, tlv: tlv::Tlv) -> Result<(), OtaError> {
        match tlv {
            tlv::Tlv::OtaType { ota_type } => {
                if ota_type != tlv::OTA_TYPE_VALUE_SSH_STAMP {
                    self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
                    return Err(OtaError::IllegalOperation);
                }
                info!("Received Ota type: {ota_type:?}");
                self.header.ota_type = Some(ota_type);
                self.tlv_holder.fill(0);
                self.current_len = 0;
            }
            tlv::Tlv::Sha256Checksum { checksum } => {
                info!("Received Checksum: {checksum:?}");
                if self.header.ota_type.is_none() {
                    error!("UpdateProcessor: Received SHA256 Checksum TLV before OTA Type TLV");
                    self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
                    return Err(OtaError::IllegalOperation);
                }
                self.header.sha256_checksum = Some(checksum);
                self.tlv_holder.fill(0);
                self.current_len = 0;
            }
            tlv::Tlv::FirmwareBlob { size } => {
                self.handle_firmware_blob(size).await?;
            }
        }
        Ok(())
    }

    async fn handle_firmware_blob(&mut self, size: u32) -> Result<(), OtaError> {
        info!("Received FirmwareBlob size: {size:?}");
        if self.header.ota_type.is_none() {
            error!("UpdateProcessor: Received FirmwareBlob TLV before OTA Type TLV");
            self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
            return Err(OtaError::IllegalOperation);
        }

        if self.header.sha256_checksum.is_none() {
            error!("UpdateProcessor: Received FirmwareBlob TLV before SHA256 Checksum TLV");
            self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
            return Err(OtaError::IllegalOperation);
        }
        let max_size = W::get_ota_partition_size()
            .await
            .map_err(|_| OtaError::InternalError)?;
        if size > max_size {
            error!(
                "UpdateProcessor: Firmware blob size {size} exceeds OTA partition size {max_size}"
            );
            self.state = UpdateProcessorState::Error(OtaError::IllegalOperation);
            return Err(OtaError::IllegalOperation);
        }
        self.header.firmware_blob_size = Some(size);

        info!("Starting OTA update");
        self.state = UpdateProcessorState::Downloading {
            total_received_size: 0,
        };
        info!("Transitioning to Downloading state");
        Ok(())
    }

    async fn process_downloading(
        &mut self,
        source: &mut tlv::TlvsSource<'_>,
        total_received_size: &mut u32,
    ) -> Result<(), OtaError> {
        let Some(total_blob_size) = self.header.firmware_blob_size else {
            error!("UpdateProcessor: Firmware blob size not set before downloading");
            return Err(OtaError::IllegalOperation);
        };
        debug!("source contains {} bytes", source.remaining());

        if *total_received_size >= total_blob_size {
            error!(
                "UpdateProcessor: Received more data than expected: received_size = {total_received_size}, total_blob_size = {total_blob_size}"
            );
            return Err(OtaError::IllegalOperation);
        }

        let to_take = source
            .remaining()
            .min((total_blob_size - *total_received_size) as usize);

        let data_chunk = source.take(to_take).map_err(|e| {
            error!("UpdateProcessor: Error taking data chunk of size {to_take}: {e:?}");
            OtaError::InternalError
        })?;

        self.hasher.update(data_chunk);

        debug!(
            "Writing {} bytes to flash at offset {}",
            data_chunk.len(),
            *total_received_size
        );
        self.ota_writer
            .write_ota_data(*total_received_size, data_chunk)
            .await
            .map_err(|e| {
                error!(
                    "UpdateProcessor: Error writing data chunk to flash at offset {}: {e:?}",
                    *total_received_size
                );
                OtaError::WriteError
            })?;

        *total_received_size += u32::try_from(to_take).map_err(|_| {
            error!("UpdateProcessor: Data chunk size overflow");
            OtaError::InternalError
        })?;

        if *total_received_size >= total_blob_size {
            self.verify_checksum()?;
            info!("All firmware data received, transitioning to Finished state");
            self.state = UpdateProcessorState::Finished {};
        } else {
            self.state = UpdateProcessorState::Downloading {
                total_received_size: *total_received_size,
            };
        }
        Ok(())
    }

    fn verify_checksum(&mut self) -> Result<(), OtaError> {
        let Some(original_hash) = self.header.sha256_checksum else {
            error!("UpdateProcessor: No original checksum to verify against after download");
            return Err(OtaError::IllegalOperation);
        };

        let computed = self.hasher.clone().finalize();
        if original_hash.as_slice() == computed.as_slice() {
            info!("UpdateProcessor: Checksum verified successfully");
        } else {
            error!(
                "UpdateProcessor: Checksum mismatch after download! Expected: {original_hash:x?}`"
            );
            self.state = UpdateProcessorState::Error(OtaError::VerificationFailed);
            return Ok(());
        }
        Ok(())
    }

    /// Finalizes the OTA update process
    ///
    /// This function should be called once all data has been processed.
    /// It will verify the final state and complete the OTA update if everything is correct.
    pub async fn finalize(&mut self) -> Result<(), OtaError> {
        let ret_val = match self.state {
            UpdateProcessorState::Finished {} => {
                info!("Finalizing OTA update process successfully.");

                self.ota_writer.finalize_ota_update().await.map_err(|e| {
                    error!("Error finalizing OTA update: {e:?}");
                    OtaError::InternalError
                })
            }
            UpdateProcessorState::Error(e) => {
                error!("Cannot finalize OTA update due to error state: {e:?}");
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

    pub fn reset_device(&mut self) {
        self.ota_writer.reset_device();
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
/// `OtaError` for OTA update processing errors
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
    /// Unknown TLV Type encountered during processing
    UnknownTlvType,
}

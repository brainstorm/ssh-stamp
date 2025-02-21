use ssh_key::{public::Ed25519PublicKey, PublicKey};
// FIXME: For demo purposes, there should be a key handler/generator on first connection.

// // Randomly created host identity.
// pub(crate) const HOST_SECRET_KEY: [u8; 32] = [
//     0xdf, 0x77, 0xbb, 0xf9, 0xf6, 0x42, 0x04, 0x40, 0x4c, 0x69, 0xe7, 0x1c, 0x7c, 0x6c, 0xda, 0x71,
//     0x6c, 0xdc, 0x20, 0xa3, 0xe1, 0x2f, 0x78, 0x4a, 0x6d, 0xaa, 0x96, 0x3a, 0x1a, 0x51, 0xea, 0x4f,
// ];

// const USER_FULL_PUBLIC_KEY: [u8; 103] = include_data!("/home/rvalls/.ssh/id_ed25519.pub");
// TODO: Handle errors in key parsing, i.e: changing a single character in the key results in esp-rs's "Overflow"
pub(crate) fn _get_user_public_key() -> Ed25519PublicKey {
    *PublicKey::from_openssh("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICWwC2CWtve93K0BubV0gf74kvzDG9WM5SfXAAcr+5dy rvalls@Romans-MBP.lan")
        .unwrap()
        .key_data()
        .ed25519()
        .unwrap()
}

pub(crate) const HOST_SECRET_KEY: &[u8; 400] = 
b"
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACD/HyNyMDvZkVWgMRzpbK6VgVk+/b627AamAjoO8T4uSAAAAJCzAcYdswHG
HQAAAAtzc2gtZWQyNTUxOQAAACD/HyNyMDvZkVWgMRzpbK6VgVk+/b627AamAjoO8T4uSA
AAAEAZYxnkyw7+ehro8oDJ2PBAO8OpJrBAezD3PLOw9CdLCP8fI3IwO9mRVaAxHOlsrpWB
WT79vrbsBqYCOg7xPi5IAAAAC2d1c0B0aGVzZXVzAQI=
-----END OPENSSH PRIVATE KEY-----
";
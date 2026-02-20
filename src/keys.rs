// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

// FIXME: For demo purposes, there should be a key handler/generator on first connection.
pub(crate) const HOST_SECRET_KEY: &[u8; 400] = b"
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACD/HyNyMDvZkVWgMRzpbK6VgVk+/b627AamAjoO8T4uSAAAAJCzAcYdswHG
HQAAAAtzc2gtZWQyNTUxOQAAACD/HyNyMDvZkVWgMRzpbK6VgVk+/b627AamAjoO8T4uSA
AAAEAZYxnkyw7+ehro8oDJ2PBAO8OpJrBAezD3PLOw9CdLCP8fI3IwO9mRVaAxHOlsrpWB
WT79vrbsBqYCOg7xPi5IAAAAC2d1c0B0aGVzZXVzAQI=
-----END OPENSSH PRIVATE KEY-----
";

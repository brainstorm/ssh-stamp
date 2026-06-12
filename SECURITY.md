<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Security Policy

This policy is designed to comply with the EU Cyber Resilience Act (CRA,
Regulation 2024/2847) and the NIS2 Directive (Directive 2022/2555), in
anticipation of ssh-stamp being embedded in commercial products placed on
the EU market.

## Product Information

- **Product name:** ssh-stamp
- **Developer:** Roman Valls Guimera — brainstorm@nopcode.org
- **Security Engineer:** Spiros Thanasoulas — dsp@2f30.org
- **License:** GPL-3.0-or-later
- **Product category:** Embedded firmware (SSH server for microcontrollers)
- **Intended use:** Network-attached SSH-to-UART bridge for IoT/embedded
  devices

## Coordinated Vulnerability Disclosure

### Reporting a vulnerability

Do **not** open a public GitHub issue for security-sensitive bugs.

Report security vulnerabilities by emailing BOTH **brainstorm@nopcode.org** and **dsp@2f30.org**.

Ideally encrypt and sign using the following PGP public keys:

https://u.2f30.org/dsp/files/pubkey.txt
http://pgp.id/pks/lookup?search=0x12A5388F08F80CB5&fingerprint=true&op=index

### What to include

- A description of the vulnerability and its potential impact.
- Steps to reproduce or a proof-of-concept.
- Any affected versions or hardware targets you have tested.
- CVSS v3.1 base score or severity estimate, if available.
- Suggested fix or mitigation, if you have one.

### Response timeline

| Phase                              | SLA                         |
|------------------------------------|-----------------------------|
| Acknowledgement of report          | 3 business days             |
| Initial triage & severity assess.  | 7 business days             |
| Critical-severity fix shipped       | 14 calendar days           |
| High-severity fix shipped           | 30 calendar days           |
| Medium/Low-severity fix shipped     | Next scheduled release      |
| Public disclosure (coordinated)    | 90 calendar days max        |

Severity is assessed using CVSS v3.1. "Shipped" means a tagged release
on the `main` branch with the fix merged and binary artifacts published.

### Disclosure policy

We follow **coordinated vulnerability disclosure**. Reporters are asked
to allow the full SLA window before public disclosure. If a fix has not
been shipped within the SLA, the reporter may disclose publicly. We
commit to keeping reporters informed of progress throughout.

We will **not** pursue legal action against researchers who act in good
faith and follow this policy.

## Incident Response and Regulatory Reporting

### CRA-compliant incident handling

In accordance with CRA Art. 13(3) and Art. 14, when an actively
exploited vulnerability is discovered in a released version of ssh-stamp:

1. **Triage:** Determine if the vulnerability is being actively
   exploited or poses a serious risk.
2. **Patch:** Develop and release a fix per the SLA above.
3. **Notify downstream manufacturers:** If ssh-stamp is embedded in a
   commercial product, the integrating manufacturer is responsible for
   propagating the fix to end users and, where required, filing
   notifications with the relevant national CSIRT per NIS2 Art. 23.
4. **SBOM update:** Publish an updated SBOM reflecting any dependency
   changes introduced by the fix.

### NIS2 obligations

ssh-stamp itself is not an operator of essential/important services under
NIS2. However, manufacturers integrating ssh-stamp into products that fall
under NIS2 must:

- Report significant incidents to their national CSIRT within **24
  hours** (early warning) and **72 hours** (incident notification), per
  NIS2 Art. 23.
- Maintain documentation of the vulnerability handling measures applied,
  per NIS2 Art. 21(2)(d).

This policy and its artifact trail (advisories, commits, SBOMs) are
intended to satisfy the upstream portion of those obligations.

## Supported Versions and Product Lifetime

| Version line | Security fixes        | End of security support |
|--------------|-----------------------|--------------------------|
| Latest release on `main` | Full support | 5 years from initial release, or until a successor version is designated, whichever is later |
| Prior major version | Best-effort | 1 year after successor release |

The **expected product lifetime** for CRA Art. 10(2)(f) purposes is **5
years** from the date of first release of each major version.

## Software Bill of Materials (SBOM)

In accordance with CRA Art. 13(2), an SBOM is generated for each release
and published alongside binary artifacts. The SBOM is produced in
SPDX format (`sbom.spdx.json`) and covers all direct and transitive Rust
dependencies. SPDX is the format supported by `sbom-cve-check` for
continuous vulnerability monitoring.

Integrating manufacturers may use the SBOM to comply with CRA Art.
13(2)(c) (vulnerability monitoring of components).

## Security Risk Assessment

ssh-stamp follows a risk assessment consistent with NIS2 Art. 21(2)(d).
The key risk categories and mitigations are:

| Risk category | Threat | Mitigation |
|---------------|--------|------------|
| Authentication | Brute-force or credential compromise | SSH key-only auth via `sunset`; no password auth |
| Network exposure | Unauthorized remote access | WiFi AP / Ethernet port is the only attack surface; no open inbound ports beyond SSH (port 22) |
| Firmware integrity | Malicious OTA update | OTA images are validated before flashing (see `ota/README.md`) |
| Supply chain | Vulnerable upstream dependency | SBOM published per release; `cargo audit` in CI |
| Data confidentiality | UART traffic interception | SSH encryption protects UART-to-SSH bridge traffic |
| Availability | DoS via network | No explicit rate-limiting; relies on MCU resource constraints as a natural throttle |

This assessment is reviewed at each major release.

## Audit Trail

All vulnerability reports, triage decisions, and fix commits are recorded
in:

- The project's **GitHub Security Advisories** (private until disclosure).
- **Git history** — fix commits reference the advisory and CVE, if
  assigned.
- Release notes document all security-relevant changes.

This log satisfies CRA Art. 13(3) documentation requirements and
supports NIS2 Art. 21(2)(d) evidence of risk management measures.

## Scope

This policy covers the ssh-stamp codebase, including the core library
(`ssh-stamp`), the HAL trait crate (`ssh-stamp-hal`), and all platform
port crates (`ssh-stamp-esp32`, etc...). 

For vulnerabilities in third-party dependencies (e.g. `sunset`,
`embassy-net`, `esp-hal`), we monitor through `cargo audit` and will
issue advisories when affected versions are in use, per CRA Art. 13(2)(c).
Upstream vulnerabilities should also be reported directly to their
respective maintainers.

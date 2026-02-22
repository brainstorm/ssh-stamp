takentaal v1.0



# SSH Stamp



The aim of this project (available at https://github.com/brainstorm/ssh-stamp) is to write software that executes code on a microprocessor and allows transferring data between the internet and various electronic interfaces. In other words: a logic "bridge" between the internet and low level electronic interfaces.



In its conception, the SSH Stamp is a secure bridge between wireless and a serial port (ubiquitous interface also known as "UART"), implemented in the Rust programming language. The Rust programming language offers convenient memory safety guarantees that are meaningful for this project from a systems security perspective.



From a practical standpoint, SSH Stamp allows its users (from different levels of expertise) to audit, monitor, defend, operate, understand and maintain a variety of low level electronics remotely (and securely) through the internet at large.



## {240} Password-based authentication (GH issue #20)



Currently, SSH Stamp accepts ANY arbitrary password when logging in via the SSH protocol. This is a well known and big security issue that requires attention to be put on the password authentication handler.



On the server side, there should be a persistent setting that completely disables password-based authentication (only accepting safer key-based authentication method (see related GH issues #21 and #23)).



- {120} Implement password authentication server-side toggle.

- {120} Implement handler business logic and connect it with GH issue #23.



## {400} Have the server generate a unique server keypair and store it persistently (on boot) (GH issue #38)



Currently, SSH Stamp accepts any (hardcoded) private key, as implied in the top-level documentation. This is a well known and big security issue that requires attention to be put on the public/private key authentication handler.



Server keys are used by the client to identify the server in a secure way. Ideally those keys should be generated at first boot and optionally re-generated if the user desires to do so.



- {400} Implement in-device private key generation and storage, test and audit its logic.



## {200} Public key-based authentication (GH issue #21)



The user should be able to provide concatenate public keys as "authorized_keys" via environment variable(s) (see related GH issue #23).



- {200} Implement pubkey authentication environment variable provisioning.



## {1000} Provisioning based on (SSH) environment variables (GH issue #23)



Many SSH clients pass environment variables to the server for many purposes.



This task is meant to exploit this commonly used pattern to simplify provisioning of new SSH Stamps. Environment variables will be used to, among other functions:



- {200} Define and persist the UART bridge parameters such as BAUD speed, stop bits.

- {200} Compile a spreadsheet of commercially available "stamp boards" and set a default, unused UART physical pins pair. Allow the user to alter that default via SSH env settings (see GH issue #23).

- {200} Pass the password and store it for subsequent sessions, disallowing un-authenticated password changes after first initialization.

- {200} Pass the private key(s) and/or trigger in-device private key generation.

- {200} Have an environment variable (or command) that instructs the processor to factory reset all non-volatile storage but keeps the firmware intact.



This approach avoids having complicated device-programming-time deployment(s) of sensitive cryptographic material.



Furthermore, provisioning the device this way yields a superior user experience: a device can be shipped on an unprovisioned state and subsequently be **NON INTERACTIVELY** provisioned via SSH env vars.



The emphasis on non-interactivity (and idempotency) is due to time tested patterns on the IT automation industry (see: Ansible, Puppet, Chef, SaltStack, etc...).



## {1700} Firmware update via SFTP (GH issue #24)



Instead of regular "out of band" firmware OTA update mechanisms, SSH Stamp firmware will be updated via its own secure protocol: SCP/SFTP.



The reason behind this approach is that we can leverage the existing SSH server AAA mechanisms to deploy new firmware securely.



Ideally, this process shouldn't have to involve the bootloader since this should happen at a "userspace + reboot" level.



- {1300} Implement SCP/SFTP as per IETF RFCs. Only the functionality needed to support firmware update.

- {300} Implement OTA mechanism on device.

- {100} Audit and thoroughly check that new firmware cannot be altered pre-auth.



The changes described above will most likely need to be applied to SSH Stamp's main current "core" dependency: "sunset". This will therefore constitute an external (to SSH Stamp) OSS contribution that will be used in our software.



## {2000} FSMs + SansIO refactor (GH issue #25)



This is a relatively big refactor but needed pre-requisite to introduce other low level I/O (protocols) beside UART such as SPI/I2C/CAN... it is also a research task since there's uncertainty on whether such a bridge between a (relatively slow) network stack and (generally faster) electrical protocols will tolerate strict timing demands.



Moving to compile-time validated finite state machines (FSMs) will hopefully make pre-auth attacks less likely to be successful on SSH Stamp. Such attacks have been unfortunately common in recent SSH server explorations, see:



https://threatprotect.qualys.com/2025/04/21/erlang-otp-ssh-server-remote-code-execution-vulnerability-cve-2025-32433/



https://www.runzero.com/blog/sshamble-unexpected-exposures-in-the-secure-shell/



The trigger of this refactoring idea is https://www.firezone.dev/blog/sans-io



This approach should ideally be combined with some kind of performance profiling to ensure we don't regress after incorporating those changes.



- {1100} Implement a PoC state machine on std outside SSH Stamp, validate corner cases there.

- {500} Adapt PoC for no_std usage and structure.

- {400} Refactor codebase for SansIO + FSM.



## {900} General performance engineering (GH issue #28)



Profiling and optimisation are tangentially mentioned towards the end of issue #25, but better tooling/process is needed to maximise the performance of this firmware on different fronts:



- {100} Choose appropriate overall and per-task heap size allocations, i.e: bisect heap size until finding a clear performance degradation under different loads.

- {100} Perform benchmarks for different RX/TX buffer sizes on both SSH network stack and UART packet queues.

- {100} Minimise the build for size as much as possible without discarding security checks (overflow protections).

- {300} Add CI/CD instrumentation to track all of the above over time, avoiding future regressions.

- {300} Profile hot spots across the app, including cryptographic primitives, and optimise them.



Those performance engineering techniques should be appropriately documented (preferably with examples) and ideally implemented in CI to monitor regressions.



## {1000} Documentation (GH issue #30)



Mid project, when the code based reaches relative maturity and stability, we should make a concerted effort to write and review docs at all levels: user, dev, architecture.md



- {200} Make sure onboarding (and quickstart) is fully documented and flawless.

- {200} Refer and implement an ARCHITECTURE.md document. Advice taken from Matklad's blogpost: https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html

- {120} List and document all the effects of environment variables from GH issue #23.

- {120} List and document both supported and unsupported devices and boards.

- {120} List and document ways to debug/profile/tweak firmware for different feature flags and scenarios.

- {120} Have at least one real world scenario documented.

- {120} Define clearly what's our threat model, i.e: we'll not protect against any physical attacks. OTOH, pre-auth attacks (exploit bad code logic that Rust doesn't protect against) can be the worst case scenario for SSH Stamp.



## {600} Testing (GH issue #37)



Test that the firmware compiles, has good coverage and there are no performance regressions as mentioned in other tasks. This task is a more focused effort so that there is comprehensive testing in both software and hardware.



- {200} Leverage embedded-test Rust crate for hardware testing (HIL).

- {200} Physically build a test harness with most of the supported targets so that they can be easily tested (ideally via CI/CD hooks).

- {200} Test that a IPV6 address is served over DHCPv6. ULA, DHCP-PD and other common scenarios work as they should.



## {500} Compile project for all Espressif ESP32 WiFi targets (GH issue #18)



The current project only compiles for the ESP32-C6, as in indicated in various feature flags and linker options.



Find a non-intrusive way to compile for as many Espressif targets as possible.



- {400} Implement cargo build targets for each Espressif microcontroller.

- {100} Resolve compilation issues that arise for each target.



## {800} Implement firmware support for alternative wifi enabled microcontrollers (other than Espressif) (GH issue #19)



Implementing and testing support for other microcontrollers from a completely different manufacturer.



- {200} Compile and flash SSH Stamp into this target with UART support.

- {600} Try to support WiFi (currently not supported at the upstream HAL). 



## {2000} Add mlkem768x25519 (to sunset?) and integrate into SSH-Stamp (GH issue #34)



Add mlkem768x25519 (post Quantum resistant crypto) to sunset dependency and integrate into SSH-Stamp.



- {1500} Implement and test MLKEM upstream in sunset.

- {500} Test in SSH Stamp.



## {1080} Bootloader audit and UX improvements (GH issue #35)



Some users will expect that the bridge is established against their own dev board USB2TTL converted UART0 port instead of custom discrete pins.



The current approach is to use UART1 and custom pins (soon to be GPIO9 and GPIO10 by default) so that boot messages and early SSH2UART are not intermixed.

This task will have to:



- {600} Explore how the default-shipped ESP-IDF 5.x bootloader is packaged in esp-hal's firmware.

- {240} Conditionally modify the bootloader accordingly so no boot messages are displayed when powering up the board and no println!/debug messages from the user level firmware itself are shown either. In other words: Only bridged SSH2UART messages should be conveyed.

- {120} Make sure those bootloader modifications are not an impediment to basic operations such as re-flashing the IC (i.e: perhaps espflash expects some bootloader output to trigger a flashing operation?).

- {120} Document a path towards a secure bootloader that is compatible with the HAL/device **and** respects the aforementioned points.



This exploration into the bootloader inner workings can also pave the way to assess whether improvements in secure boot are interesting to pursue and how.



## {840} Implement WiFi STA mode (GH issue #36)



Currently SSH Stamp boots up in AP (Access Point) mode and expects DHCP clients. This tasks would put SSH Stamp in STA (Station mode) or acting as a WiFi client instead.



This is attractive on more permanent installations where SSH Stamp can be part of a larger local wireless network.



- {240} Make appropriate changes to device HAL initialization.

- {240} Verify that those changes do not break prior assumptions.

- {240} Align onboarding process for that mode's particularities.

- {120} Make the setting configurable (see GH issue #23).



## {200} Licensing (GH issue #22)



There's some borrowed code from SSH Stamp's main dependency: sunset. Clarify this properly and/or find the right way to compose LICENSE wording.



- {200} Process feedback and apply changes from NLNet licensing experts.



## {2000} Implement bridging support for other electrical protocols such as: CAN, SPI and I2C.



Encapsulate transaction-oriented protocols (CAN, SPI, I2C) over a plain byte stream. This task assumes that the the processor's timing characteristics are suitable. At a protocol level, for CAN packet encapsulation there's "slcan" and "GVRET" among other more exotic encapsulation protocols.



Equivalents for SPI-over-Network or I2C-over-Network are a much more experimental and uncharted territory, perhaps the Bus Pirate command protocol would be the closest match.



- {1000} Implement slcan and/or GVRET for SSH_to_CAN support.

- {1000} Investigate and implement proof of concept for SPI and/or I2C.



## {600} Final release



Final review on all the work done and refinement, optimization and security audit handover.



- {120} Stable release.

- {240} Process feedback from security audit and accessibility scan.

- {240} Final release with updated documentation.

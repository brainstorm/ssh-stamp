# Normal OTA

TBD: Enumerate existing OTA mechanisms (compatible with esp-hal/no_std) and ideally see how big their RAM/Flash footprint is, explain pros and cons, etc.

# Custom OTA: SCP

SSH Stamp could leverage pre-existing OTA mechanisms that involve bootloaders, custom cloud services that distribute new builds to fleets and so on.

But we are writing a SSH server... wouldn't it make sense to just use the SCP protocol to transfer new builds to the device? [That's what Andrew Zonenberg has done on his StaticNet project][ssh_stamp_scp_ota].

[ssh_stamp_scp_ota]: https://github.com/brainstorm/ssh-stamp/issues/24

TBD: Describe SCP protocol, how to implement it, how to use it, how to test it, etc.
# Config via host environment variables

Before a SSH connection is established, the hosts's environment variables are read to configure the firmware, borrowing a convenient feature from the (Open)SSH client binary and protocol (courtesy of man (1) ssh):

```
    (...) ssh reads ~/.ssh/environment, and adds lines of the format “VARNAME=value” to the environment if the file exists and users are allowed to change their environment.
```

This allows for dynamic configuration of the firmware based on the host environment, enabling flexibility and adaptability in different deployment scenarios. For instance, the pin configuration for the UART TX pin can be set via an environment variable, allowing the firmware to adapt to different hardware setups without requiring editing files, performing resets nor recompiling the firmware:

```
export PIN_UART_TX=2

ssh -o "SendEnv PIN_UART_TX" user@host
```

Other environment variables can be used to configure the firmware's behavior, such as:

1. Setting the Wi-Fi SSID, password and encryption standard (Open, WPA, WPA2, etc...).
2. Setting password-based authentication for the SSH server.
3. Setting public key authentication for the SSH server (and/or generating an new server secret, adding a new (third party?) public key, etc...).
4. Enabling debugging features such as logging levels or enabling/disabling specific tasks.
5. Wiping the firmware's configuration, returning to factory defaults, [see chapter 3](chapter_3.md).

Those are a few examples of the host environment variables as firmware configuration mechanism, more can follow as SSH Stamp grows and can also be adapted for custom use cases.

# Serial Port Function Linker

Create stable function-based symlinks (for example, `program_esp32`, `com_esp32`) from `/dev/serial/by-id` entries to '/dev/serial/by-function' using a map file.


This project installs:

- `/usr/local/bin/link_serials_by_function.sh` (runtime linker)
- `/etc/systemd/system/link-serials-by-function.service` (systemd oneshot)
- `/etc/udev/rules.d/99-link-serials-by-function.rules` (udev trigger on add/remove)
- `/etc/tempfiles.d/serials-by-function.conf` (tmpfiles rule to recreate output dir on boot)
- `/etc/serials_map.conf` (map serial/by-id to function)

## Why

`/dev/ttyUSB*` numbers can change after reconnect/reboot.
`/dev/serial/by-id/*` is stable per device.
This tool maps stable IDs to role names.

## Mapping file format

`serial_id=function_name`

Example (`example_map.conf`):

```conf
# Map with the name of the serial device as found in /dev/serial/by-id/ folder and 
# the function (prog_*, com_*, other_*)
usb-1a86_USB_Single_Serial_5AE7080382-if00=prog_esp32s3
usb-FTDI_FT232R_USB_UART_AN94725E-if00-port0=com_esp32s2
usb-1a86_USB_Serial-if00-port0=prog_esp32
usb-FTDI_FT232R_USB_UART_A300V04K-if00-port0=com_esp32
usb-Espressif_USB_JTAG_serial_debug_unit_40:4C:CA:8C:E9:7C-if00=prog_esp32c3
usb-FTDI_FT232R_USB_UART_AV74K0PO-if00-port0=com_esp32c3
usb-Espressif_USB_JTAG_serial_debug_unit_F0:F5:BD:0E:60:58-if00=prog_esp32c6
usb-FTDI_FT232R_USB_UART_AT4P96AZ-if00-port0=com_esp32c6
```

## Install

From serial-by-function folder:

```bash
chmod +x ./install_serials_by_function.sh
sudo ./install_serials_by_function.sh config/serial_map.conf /dev/serial/by-function
```

## Uninstall

To remove the installed service, udev rule, tmpfiles rule, and linker script:

```bash
chmod +x deploy/uninstall_serials_by_function.sh
sudo ./deploy/uninstall_serials_by_function.sh
```

To also remove the installed mapping file (`/etc/serials_map.conf`):

```bash
sudo ./uninstall_serial_by_function.sh --remove-map
```

Optional: remove the runtime output directory if it still exists:

```bash
sudo rm -rf /dev/serial/by-function
```

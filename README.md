# usbmount

External storage devices mounting tool, helping mount and umount USB disks like [udisks2](https://www.freedesktop.org/wiki/Software/udisks/) in the command line.

## Motivation

The `polkit`, which is a dependency of `udisks2`, is a privilege escalation service like `sudo`.   
Both `sudo` and `polkit` have vulnerabilities in history and they leave a vast attack surface for attackers, but we can't simply uninstall all of them since we need privilege escalation for daily use.  
The better choice is to keep `sudo` and uninstall `polkit` because it's not as necessary as `sudo`. Therefore, this tool is made to help mount external storage devices without `udisks2` and `polkit`.

## Dependency

- sudo
- libudev (provided by systemd)
- rust (build only)
- pkgconf (build only)

## Usage

### mount

Use `usbmount mount` or `usbmount m` to select a device to mount. If there is only one USB block device, it will automatically select and mount it.  
You can also specify the device or mount path by `usbmount mount [dev_path] [mount_path]`.

### umount

The usage of `umount` is almost identical to `mount`.

### info

`usbmount info` or `usbmount i` will show current devices' mount status.

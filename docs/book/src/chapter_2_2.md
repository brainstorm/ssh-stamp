# Non Volatile Storage (NVS)

1. How should storage be structured for lightweight configuration purposes?
1. Is [cfg_noodle][cfg_noodle] a good fit for this? Or should we use [sequential_storage][sequential_storage] instead?
1. Is it worth using a custom NVS implementation for this firmware? Or should we use the existing esp-hal NVS implementation and build on top of it?
1. Is the serdes approach implemented by [sunset's][sunset] (and SSH Stamp's) SSHConfig a good fit for this? Or are there better solutions?
1. How should config be accessed/updated/deleted across tasks with the least amount of boilerplate (needing the least amount of .lock(), mutex guards, etc..)?

[cfg_noodle]: https://github.com/tweedegolf/cfg-noodle
[sequential_storage]: https://github.com/tweedegolf/sequential-storage
[sunset]: https://github.com/mkj/sunset
# Third-party notices

This project is licensed under `MIT OR Apache-2.0` (see `LICENSE-MIT` and `LICENSE-APACHE`).
It contains the following third-party material:

- **Ported logic.** `crates/ssd2683` and `crates/zectrix-board` are Rust ports of command
  sequences, pin maps and formulas from the MIT-licensed C/C++ projects below; no source
  file from them is copied verbatim.
- **Copied code.** The linker helper functions in `apps/note4/build.rs` and
  `apps/note4c/build.rs` (all of `apps/note4/build.rs`) and the `#![deny(clippy::mem_forget)]`
  block in `apps/*/src/main.rs` are taken from the esp-generate project template, with a
  provenance comment in each file.
- **Data.** `partitions/factory-16m.csv` is the NOTE4C factory partition table as published
  in youn-ink-fourcolor-firmware (from xiaozhi-esp32). The ISRG Root X1 certificate is
  listed below.

Each MIT notice is reproduced in full, as the licence requires.

## zectrix-note4-epd-demo (Zectrix Lab)

<https://github.com/itopinion/zectrix-note4-epd-demo>. Source of the NOTE4 mono panel
sequence: panel setting, OTP selection, temperature codes, the reset-before-refresh
rule, and the partial refresh (window command, 8-px alignment, transition codes).

```
MIT License

Copyright (c) 2026 Zectrix Lab

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## youn-ink-fourcolor-firmware (LazyYoun / macheng2017)

<https://github.com/LazyYoun/youn-ink-fourcolor-firmware>, ported at commit `51812e4`
(`tools/refs.txt` pins the later `8907e59`; the driver files are the same in both). Source
of the NOTE4C four-colour refresh sequence, the colour codes and
`partitions/factory-16m.csv` (`firmware/partitions.csv`). The project is based on
xiaozhi-esp32 (next section), whose copyright lines it carries.

```
MIT License

Copyright (c) 2026 macheng2017

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## xiaozhi-esp32 (78 / Shenzhen Xinzhi Future Technology)

<https://github.com/78/xiaozhi-esp32>, file
`main/boards/zectrix/zectrix-s3-epaper-4.2/zectrix-s3-epaper-4.2.cc`. Origin of the battery
percentage curve in `crates/zectrix-board/src/battery.rs` (via youn-ink-fourcolor-firmware
and emini-home), and the upstream of the youn-ink firmware and its partition table.

```
MIT License

Copyright (c) 2025 Shenzhen Xinzhi Future Technology Co., Ltd.
Copyright (c) 2025 Project Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## emini-home (Tomasz Fiedoruk)

<https://github.com/fiedoruk/emini-home> at `afd1239` (v0.6.0). Source of the pin
documentation, the BUSY-timeout and refresh-start checks, the battery validity window and
the flash-layout approach (write only `0x8000`/`0x20000`, keep the factory bootloader).

```
MIT License

Copyright (c) 2026 Tomasz Fiedoruk

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## esp-generate project template (esp-rs)

<https://github.com/esp-rs/esp-generate>, version 1.4.0, dual-licensed MIT / Apache-2.0.
Copied: the build-script linker helpers (`check_xtensa_linker_available`, `xtensa_linker`,
`cargo_configured_linker`, `cargo_linker_env_var`, `linker_be_nice` and the linker lines in
`main`) and the `#![deny(clippy::mem_forget, reason = …)]` attribute.

```
MIT License

Copyright (c) 2024 esp-rs

Permission is hereby granted, free of charge, to any person obtaining a copy of this
software and associated documentation files (the "Software"), to deal in the Software
without restriction, including without limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## esp-hal and esp-mbedtls examples (esp-rs)

<https://github.com/esp-rs/esp-hal> (`examples/wifi/embassy_dhcp`, tag `esp-hal-v1.2.2`) and
<https://github.com/esp-rs/esp-mbedtls> (`examples/esp`, commit `e0b0611`), both dual-licensed
MIT / Apache-2.0. `crates/zectrix-net` (`wifi.rs`, `https.rs`) follows their structure for the
Wi-Fi station, embassy-net and mbedtls-rs session setup. The code was written for this
project after reading the examples, not copied file by file; as it follows them closely,
the notice is kept here.

esp-hal, `LICENSE-MIT` at tag `esp-hal-v1.2.2`:

```
Copyright 2021 esp-rs

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

esp-mbedtls, `LICENSE-MIT` at commit `e0b0611`:

```
MIT License

Copyright (c) 2022 esp-rs

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ISRG Root X1 (Let's Encrypt)

`crates/zectrix-net/certs/isrgrootx1.pem` is the public ISRG Root X1 certificate from
<https://letsencrypt.org/certs/isrgrootx1.pem>, used as the TLS trust anchor.

## Vendor documents (not included)

The wiki pages, schematic, datasheets, images and firmware images used for the hardware
research belong to their owners: ZECTRIX / Zectrix Lab, Solomon Systech, Espressif, NXP and
the authors of the community projects. They are **not** part of this repository.
`tools/vendor-files.txt` and `tools/vendor-pages.txt` list where to get them;
`just fetch-vendor` downloads the files into the git-ignored `docs/vendor/` for private
reference. `docs/vendor/RESEARCH_NOTES.md` is this project's own writing.

LitoMore/trmnl-firmware is **GPL-3.0**; it was read for reference only, and no code or text
from it is in this repository. Do not copy from it, from transitink-os (PolyForm-NC) or from
zectrix-rs (no licence).

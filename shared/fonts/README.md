# Native Satoshi fonts

Caper's native clients use the same Satoshi family as the web client. The font
binaries are not stored in this repository because the ITF Free Font License
does not permit standalone redistribution through repositories.

Acquire the official, unmodified OTF files with:

```sh
python3 scripts/native_fonts.py
```

This downloads the official Fontshare family package into the ignored
`shared/fonts/cache/` directory and verifies pinned SHA-256 hashes of all four
font files and the accompanying license before caching or extracting them.
Running the command downloads licensed font software
and constitutes acceptance of the license.

Official download: `https://api.fontshare.com/v2/fonts/download/satoshi`

Fontshare's `Satoshi_Complete.zip` container varies across downloads/regions.
The build pins the embedded resources, not the ZIP's packaging metadata.
Downloads are bounded to 8 MiB and each extracted member to 1 MiB. Unused ZIP
members are never extracted. Altered fonts or license terms fail closed.

| Cached resource | Weight | PostScript name | SHA-256 |
| --- | ---: | --- | --- |
| `Satoshi-Regular.otf` | 400 | `Satoshi-Regular` | `711c6243cdc5431f9cc966e4de18bfb940365bad81acffd1e7948dbe3f254386` |
| `Satoshi-Medium.otf` | 500 | `Satoshi-Medium` | `93330866d109f6b2e298748958ec6fa4010cacef586783f281a0b268cab7fc6e` |
| `Satoshi-Bold.otf` | 700 | `Satoshi-Bold` | `50e4f9b7c1864c50761d729d6001bfac708c80457fa6fc41559a8ab1bd2573ff` |
| `Satoshi-Black.otf` | 900 | `Satoshi-Black` | `49bdb8b9436b8b9192f6f14b7ce4b96d1a3822e13c504c00c0b2842357d265cc` |

Source: [Satoshi on Fontshare](https://www.fontshare.com/fonts/satoshi),
designed by Indian Type Foundry. The package contains
`Satoshi_Complete/License/FFL.txt`, ITF Free Font License version 2.0,
17 Aug 2026. The license permits embedding in mobile and desktop applications.
It prohibits modifying, subsetting, or converting the fonts and prohibits
redistributing the standalone font software. Platform builds must copy these
files unchanged into application resources and must not publish the cache or
the font files as separate artifacts. The helper also extracts the unchanged
license as `shared/fonts/cache/Satoshi-FFL.txt`; bundle it with the application.
Its SHA-256 is `145e7fe2429a3336ba215c070ef722000e01348a3e1baaa127e871bb5012f554`.
See the
[current license page](https://www.fontshare.com/licenses/itf-ffl).

Run the focused integrity/refusal tests with:

```sh
python3 shared/fonts/test_native_fonts.py -v
```

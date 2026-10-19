# Third-party licenses

fresnel is under MIT OR Apache-2.0 ([README](../README.md#license)); parts of others keep theirs.

## Files of the repository

- `crates/fresnel-ui/fonts/JetBrainsMono-Regular.ttf`: JetBrains Mono 2.304, copyright 2020 The
  JetBrains Mono Project Authors, under the SIL Open Font License 1.1, whose text is `OFL.txt`
  next to it. The app embeds the font unchanged; its name table keeps the copyright and the license.
- `crates/fresnel-core/fixtures/terminal`: the output of busybox in a container of the tests, not
  a part of busybox. `fixtures/k3s` and `fixtures/kinds`: objects of the scenes of the tests on k3s.
- The icon and the pictures of `docs/screenshots` come from the code of this repository.

## Crates

`THIRD-PARTY-LICENSES`, in `Fresnel.app/Contents/Resources` and next to `fresnel.exe`, has the
license texts of each crate of the app (`cargo xtask notices`). Each one has a permissive license
(MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0, Unicode-3.0) or a choice with one. To note:
- `imbl` and `imbl-sized-chunks`, the maps of the stores: MPL-2.0, a copyleft of each file. The
  build uses them unchanged from crates.io, which has their source; a changed file stays MPL-2.0.
- `epaint_default_fonts`, the fonts of egui in the app: Hack (MIT and the Bitstream Vera License),
  Ubuntu Light (Ubuntu Font Licence 1.0), Noto Emoji (OFL 1.1) and emoji-icon-font (MIT).
- `self_cell` is Apache-2.0 OR GPL-2.0-only, and `r-efi` (only for UEFI) MIT OR Apache-2.0 OR
  LGPL-2.1-or-later: fresnel takes the Apache-2.0 or the MIT choice.

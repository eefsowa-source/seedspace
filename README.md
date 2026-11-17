# Seedspace

Seed-driven reverb/delay audio plugin for macOS. Each seed chooses the internal space; the controls shape its size, decay, diffusion, tone, motion, and spectral character. Builds as VST3, CLAP, and a standalone app.

## Build and test

Requires macOS and a stable Rust toolchain.

```sh
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Package

`scripts/make-pkg.sh` builds a release and writes `dist/Seedspace-<version>.pkg`.

```sh
bash scripts/make-pkg.sh
```

The installer places:

- `/Library/Audio/Plug-Ins/VST3/Seedspace.vst3`
- `/Library/Audio/Plug-Ins/CLAP/Seedspace.clap`
- `/Library/Audio/Plug-Ins/Seedspace/LICENSE`

The pkg is arm64-only, ad-hoc signed, and not notarized. macOS Gatekeeper may block it, especially on another Mac. A Developer ID signature and notarization are required for normal distribution.

## License

Seedspace is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the full text.

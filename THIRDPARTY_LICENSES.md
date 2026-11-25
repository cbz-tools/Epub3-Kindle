# Third-Party Licenses

This project uses third-party Rust crates. Exact dependency versions are recorded in
`Cargo.lock`.

## Runtime dependencies

These direct dependencies are used by the shipped library and CLI:

| Crate | Version | License |
|---|---:|---|
| `flate2` | 1.1.10 | MIT OR Apache-2.0 |
| `image` | 0.25.9 | Apache-2.0 OR MIT |
| `quick-xml` | 0.37.5 | MIT |
| `thiserror` | 2.0.20 | Apache-2.0 OR MIT |
| `unicode-normalization` | 0.1.25 | MIT OR Apache-2.0 |
| `zip` | 2.4.2 | MIT |

## Build-only dependencies

These direct dependencies are used only while building the project and are not shipped
runtime components:

| Crate | Version | License |
|---|---:|---|
| `winresource` | 0.1.31 | MIT |

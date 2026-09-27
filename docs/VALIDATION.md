# Validation — 0.1.0

Verified locally on Windows x64 with Rust 1.98.1 and the `x86_64-pc-windows-gnu` toolchain.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed, no warnings |
| `cargo test --locked` | 10 filesystem/classification tests passed |
| `cargo build --release --locked` | Passed; standalone Windows executable produced |
| Runtime dependencies | Executable imports Windows system DLLs; no separate MinGW DLL required |
| Launch | Native window rendered successfully |
| Native file picker | Three generated sample files selected together |
| Destination picker / preview | Source and destination displayed; PDF/JPG/ZIP mapped to Documents/Images/Archives |
| Confirmation / move | All three sample files moved; filesystem checked |
| Undo from GUI | All three restored; destination folders contained no files afterwards |

The automated suite covers all seven categories, case-insensitive extensions, batch duplicate names, repeated selections, non-file rejection, already-sorted files, Unicode names, stale destination conflicts, partial batches, edited files, recreated originals, locked source files, empty files, and retrying Undo after a missing original parent is recreated.

`screenshot.png` is a real capture of the running application using generated demonstration files. The screenshot is from the GUI smoke test; the final build additionally enables accessibility support and validates the original parent folder before Undo.

Not manually exercised: Explorer drag-and-drop gestures, physical cross-drive moves, removable/network drives, power-loss recovery, and Windows versions other than the local test machine. The app uses eframe's native dropped-file events. Cross-drive transfers use Windows copy/delete semantics and are not atomic. Undo history is intentionally limited to the current session.

GitHub Actions is configured to independently run formatting, linting, tests, and a release build on Windows with the MSVC toolchain. Its status should be checked on the repository rather than inferred from this local report.

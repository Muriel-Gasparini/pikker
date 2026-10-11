#!/bin/sh
# Linux cross-build only. Never installs toolchains or changes system packages.
set -eu
task_target="${1:-}"
case "$task_target" in
    x86_64-pc-windows-gnu|x86_64-pc-windows-gnullvm) ;;
    *) printf '%s\n' 'Usage: sh scripts/build-windows.sh x86_64-pc-windows-{gnu,gnullvm}' >&2; exit 2 ;;
esac
if ! rustup target list --installed | rg -Fx "$task_target" >/dev/null; then
    printf '%s\n' "Missing Rust target: $task_target. Install it explicitly with rustup target add." >&2
    exit 2
fi
if [ -n "${PIKKER_WINDOWS_TOOLCHAIN:-}" ]; then
    task_linker="$PIKKER_WINDOWS_TOOLCHAIN/bin/x86_64-w64-mingw32-clang"
    PATH="$PIKKER_WINDOWS_TOOLCHAIN/bin:$PATH"
    export PATH
else
    case "$task_target" in
        x86_64-pc-windows-gnu) task_linker="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}" ;;
        *) task_linker="${CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER:-x86_64-w64-mingw32-clang}" ;;
    esac
fi
case "$task_target" in
    x86_64-pc-windows-gnu)
        export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$task_linker"
        export CC_x86_64_pc_windows_gnu="$task_linker"
        ;;
    *)
        export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$task_linker"
        export CC_x86_64_pc_windows_gnullvm="$task_linker"
        ;;
esac
if [ -n "${PIKKER_WINDOWS_TOOLCHAIN:-}" ]; then
    export AR_x86_64_pc_windows_gnu="$PIKKER_WINDOWS_TOOLCHAIN/bin/llvm-ar"
    export AR_x86_64_pc_windows_gnullvm="$PIKKER_WINDOWS_TOOLCHAIN/bin/llvm-ar"
fi
if ! command -v "$task_linker" >/dev/null; then
    printf '%s\n' 'Missing MinGW linker. Set PIKKER_WINDOWS_TOOLCHAIN to an explicitly installed LLVM-MinGW directory.' >&2
    exit 2
fi
cargo build --release --locked --target "$task_target" --bin pikker
mkdir -p dist
cp "target/$task_target/release/pikker.exe" dist/pikker.exe
if [ "$task_target" = x86_64-pc-windows-gnullvm ] && [ -n "${PIKKER_WINDOWS_TOOLCHAIN:-}" ]; then
    cp "$PIKKER_WINDOWS_TOOLCHAIN/x86_64-w64-mingw32/bin/libunwind.dll" dist/libunwind.dll
    cp "$PIKKER_WINDOWS_TOOLCHAIN/LICENSE.TXT" dist/LLVM-LICENSE.txt
    cp "$PIKKER_WINDOWS_TOOLCHAIN/x86_64-w64-mingw32/share/mingw32/COPYING.MinGW-w64-runtime.txt" dist/MinGW-runtime-LICENSE.txt
    cp "$PIKKER_WINDOWS_TOOLCHAIN/x86_64-w64-mingw32/share/mingw32/COPYING.MinGW-w64.txt" dist/MinGW-LICENSE.txt
    sha256sum dist/libunwind.dll
fi
sha256sum dist/pikker.exe
printf '%s\n' 'Inspect DLL imports before distribution. Cross-build does not validate a real Windows LoL session.'

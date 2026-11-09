# Building and installing Toot

Toot is being prepared for beta testing. Build from source on a Linux desktop with a working Secret Service keyring (such as GNOME Keyring or KWallet with Secret Service enabled).

## Build

Install Rust stable with Cargo and the `just` command runner. On Ubuntu/Pop!_OS, install the native dependencies:

```sh
sudo apt install build-essential pkg-config libssl-dev libsecret-1-dev \
  libxkbcommon-dev libwayland-dev libudev-dev libinput-dev libdbus-1-dev \
  libfontconfig-dev libfreetype-dev libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev
cargo build --release --locked
```

Cargo downloads the locked Rust dependencies, including libcosmic. For video playback, also install:

```sh
sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-libav
```

Run `cargo run --locked` to try a development build. A graphical desktop and working keyring are needed for sign-in and saved accounts.

## Install

For your user only:

```sh
just prefix="$HOME/.local" install
```

Ensure `~/.local/bin` is on your `PATH`. For a system-wide installation, run `sudo just install`. To uninstall, use `just prefix="$HOME/.local" uninstall` or `sudo just uninstall` with the same prefix used for installation.

The installer includes the binary, desktop launcher, SVG icon, and AppStream metadata. It does not change the keyring or remove your drafts.

To prepare a staging directory for packaging without installing on your computer:

```sh
just stage /tmp/toot-package
```

The resulting tree is rooted at `/tmp/toot-package/usr`. The binary is dynamically linked: packages must declare native library dependencies and target the distribution/architecture used to build it. No standalone binary or universal Linux package is implied.

## Check changes

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked
just check-resources
```

Resource checks need Python 3, `desktop-file-utils`, and `appstream`. CI runs these checks and verifies a staged development installation. See [beta testing](BETA_TESTING.md) for checks that need a desktop and a real account.

#!/usr/bin/env bash
set -euo pipefail

# --- Helpers ---

bold=""
reset=""
if command -v tput &>/dev/null && [ -t 1 ]; then
    bold=$(tput bold)
    reset=$(tput sgr0)
fi

info()  { echo "${bold}==> $1${reset}"; }
step()  { echo; info "$1"; }
warn()  { echo "  warning: $1"; }
fail()  { echo "  error: $1" >&2; exit 1; }

# --- Detect existing installation ---

upgrading=0
if command -v cue &>/dev/null; then
    existing_version=$(cue --version 2>/dev/null || echo "unknown")
    upgrading=1
fi

# --- Prerequisites ---

step "Checking prerequisites"

if ! command -v cargo &>/dev/null; then
    echo "  Rust is not installed."
    read -rp "  Install Rust via rustup? [Y/n] " ans
    case "${ans:-Y}" in
        [Yy]*|"")
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            # shellcheck source=/dev/null
            source "$HOME/.cargo/env"
            ;;
        *) fail "cargo is required to build cue" ;;
    esac
fi

echo "  cargo: $(cargo --version)"

# --- Build ---

if [ "$upgrading" = "1" ]; then
    step "Upgrading cue (current: $existing_version)"
else
    step "Building cue"
fi

cargo build --release
echo "  Build complete."

# --- Install binary ---

step "Installing binary"

binary="$PWD/target/release/cue"

default_dir="$HOME/.local/bin"
if [ "$upgrading" = "1" ]; then
    existing_path=$(command -v cue)
    default_dir=$(dirname "$existing_path")
elif [ -d "$HOME/.cargo/bin" ] && [[ ":$PATH:" == *":$HOME/.cargo/bin:"* ]]; then
    default_dir="$HOME/.cargo/bin"
fi

read -rp "  Install to [$default_dir]: " install_dir
install_dir="${install_dir:-$default_dir}"
install_dir="${install_dir/#\~/$HOME}"

mkdir -p "$install_dir"
if [ -f "$install_dir/cue" ]; then
    echo "  Replacing existing binary at $install_dir/cue"
fi
cp "$binary" "$install_dir/cue"
chmod 755 "$install_dir/cue"
echo "  Installed to $install_dir/cue"

if [[ ":$PATH:" != *":$install_dir:"* ]]; then
    warn "$install_dir is not in your PATH"
    echo "  Add it with: export PATH=\"$install_dir:\$PATH\""
fi

# --- Spotify credentials ---

step "Spotify app setup"

case "$OSTYPE" in
    darwin*) config_dir="$HOME/Library/Application Support/cue" ;;
    *)       config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cue" ;;
esac

config_file="$config_dir/config.toml"
write_config=1

if [ -f "$config_file" ]; then
    echo "  Config already exists: $config_file"
    if [ "$upgrading" = "1" ]; then
        echo "  Keeping existing config."
        write_config=0
    else
        read -rp "  Overwrite? [y/N] " overwrite
        case "${overwrite:-N}" in
            [Yy]*) ;;
            *) echo "  Keeping existing config."; write_config=0 ;;
        esac
    fi
fi

if [ "$write_config" = "1" ]; then
    echo
    echo "  Create a Spotify app to get your credentials:"
    echo "    1. Go to ${bold}https://developer.spotify.com/dashboard${reset}"
    echo "    2. Create a new app"
    echo "    3. Set the redirect URI to ${bold}http://127.0.0.1:8888/callback${reset}"
    echo "    4. Copy the Client ID and Client Secret"
    echo

    read -rp "  Client ID: " client_id
    if [ -z "$client_id" ]; then
        fail "Client ID cannot be empty"
    fi
    if [[ ! "$client_id" =~ ^[a-zA-Z0-9]+$ ]]; then
        fail "Client ID should contain only alphanumeric characters"
    fi

    read -rsp "  Client Secret: " client_secret
    echo
    if [ -z "$client_secret" ]; then
        fail "Client Secret cannot be empty"
    fi
    if [[ ! "$client_secret" =~ ^[a-zA-Z0-9]+$ ]]; then
        fail "Client Secret should contain only alphanumeric characters"
    fi

    mkdir -p "$config_dir"
    (
        umask 077
        cat > "$config_file" <<EOF
[spotify]
client_id = "$client_id"
client_secret = "$client_secret"
EOF
    )
    echo "  Config written to $config_file"
fi

# --- Shell completions ---

step "Shell completions"

cue_bin="$install_dir/cue"

read -rp "  Generate shell completions? [Y/n] " gen_completions
case "${gen_completions:-Y}" in
    [Yy]*|"")
        echo "  Shells: bash, zsh, fish"
        read -rp "  Shell [bash]: " shell_choice
        shell_choice="${shell_choice:-bash}"

        case "$shell_choice" in
            bash)
                comp_file="$HOME/.local/share/bash-completion/completions/cue"
                mkdir -p "${comp_file%/*}"
                "$cue_bin" completions bash > "$comp_file"
                echo "  Written to $comp_file"
                echo "  Run: source $comp_file"
                ;;
            zsh)
                comp_dir="${ZDOTDIR:-$HOME}/.zfunc"
                mkdir -p "$comp_dir"
                "$cue_bin" completions zsh > "$comp_dir/_cue"
                echo "  Written to $comp_dir/_cue"
                echo "  Ensure $comp_dir is in your fpath and run: compinit"
                ;;
            fish)
                comp_dir="$HOME/.config/fish/completions"
                mkdir -p "$comp_dir"
                "$cue_bin" completions fish > "$comp_dir/cue.fish"
                echo "  Written to $comp_dir/cue.fish"
                ;;
            *)
                warn "Unknown shell: $shell_choice (skipping)"
                ;;
        esac
        ;;
    *) echo "  Skipped." ;;
esac

# --- Authenticate ---

if [ "$upgrading" = "1" ]; then
    step "Authentication"
    echo "  Existing auth preserved. Skipping."
else
    step "Authentication"

    read -rp "  Authenticate with Spotify now? [Y/n] " do_auth
    case "${do_auth:-Y}" in
        [Yy]*|"")
            echo "  Running: cue devices"
            echo "  Your browser will open for Spotify authorization."
            echo
            "$cue_bin" devices || warn "Authentication did not complete. Run 'cue devices' later to retry."
            ;;
        *)
            echo "  Skipped. Run any cue command later to trigger authentication."
            ;;
    esac
fi

# --- Done ---

if [ "$upgrading" = "1" ]; then
    step "Upgrade complete"
else
    step "Setup complete"
fi
echo
echo "  Quick start:"
echo "    cue devices          List available devices"
echo "    cue play <query>     Play a track"
echo "    cue now              Show what's playing"
echo "    cue --help           See all commands"
echo

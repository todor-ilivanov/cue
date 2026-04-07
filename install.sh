#!/usr/bin/env bash
set -euo pipefail

# --- Options ---

auto_yes=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes) auto_yes=1 ;;
        -h|--help)
            echo "Usage: install.sh [--yes]"
            echo "  -y, --yes    Accept defaults and skip prompts"
            exit 0
            ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

# --- Helpers ---

bold=""
reset=""
yellow=""
red=""
if command -v tput &>/dev/null && [ -t 1 ]; then
    bold=$(tput bold)
    reset=$(tput sgr0)
    yellow=$(tput setaf 3)
    red=$(tput setaf 1)
fi

info()  { echo "${bold}==> $1${reset}"; }
step()  { echo; info "$1"; }
warn()  { echo "  ${yellow}warning:${reset} $1"; }
fail()  { echo "  ${red}error:${reset} $1" >&2; exit 1; }

# Prompt with a default. In --yes mode, returns the default silently.
ask() {
    local prompt="$1" default="$2" var="$3"
    if [ "$auto_yes" = "1" ]; then
        eval "$var=\"$default\""
        return
    fi
    read -rp "$prompt" reply
    eval "$var=\"\${reply:-$default}\""
}

# Prompt for a yes/no. In --yes mode, returns the default.
confirm() {
    local prompt="$1" default="$2"
    if [ "$auto_yes" = "1" ]; then
        [ "$default" = "Y" ]
        return
    fi
    local reply
    read -rp "$prompt" reply
    case "${reply:-$default}" in
        [Yy]*|"") return 0 ;;
        *) return 1 ;;
    esac
}

cleanup=""
trap 'if [ -n "$cleanup" ]; then rm -f "$cleanup"; fi' EXIT

# --- Detect existing installation ---

is_our_cue() {
    "$1" --help 2>&1 | head -1 | grep -q "Spotify remote control"
}

upgrading=0
existing_path=""
if command -v cue &>/dev/null; then
    candidate=$(command -v cue)
    if is_our_cue "$candidate"; then
        existing_path="$candidate"
        existing_version=$("$candidate" --version 2>/dev/null || echo "unknown")
        upgrading=1
    else
        warn "Found '$candidate' but it is not cue (Spotify). Installing alongside it."
    fi
fi

# --- Prerequisites ---

step "Checking prerequisites"

if ! command -v cargo &>/dev/null; then
    echo "  Rust is not installed."
    if confirm "  Install Rust via rustup? [Y/n] " Y; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    else
        fail "cargo is required to build cue"
    fi
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

binary="$PWD/target/release/cue"
if ! "$binary" --version &>/dev/null; then
    fail "Built binary failed to run. Check build output above."
fi

# --- Install binary ---

step "Installing binary"

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"

if [ "$upgrading" = "1" ]; then
    default_dir=$(dirname "$existing_path")
elif [ -d "$cargo_bin" ] && [[ ":$PATH:" == *":$cargo_bin:"* ]]; then
    default_dir="$cargo_bin"
else
    default_dir="$HOME/.local/bin"
fi

ask "  Install to [$default_dir]: " "$default_dir" install_dir
install_dir="${install_dir/#\~/$HOME}"

mkdir -p "$install_dir"
if [ -f "$install_dir/cue" ]; then
    echo "  Replacing existing binary at $install_dir/cue"
fi
tmpbin=$(mktemp "$install_dir/cue.XXXXXX")
cleanup="$tmpbin"
cp "$binary" "$tmpbin"
chmod 755 "$tmpbin"
mv "$tmpbin" "$install_dir/cue"
cleanup=""
echo "  Installed to $install_dir/cue"

if command -v cue &>/dev/null; then
    resolved=$(command -v cue)
    if [ "$resolved" != "$install_dir/cue" ]; then
        warn "Another 'cue' exists at $resolved and takes precedence in PATH"
        warn "Ensure $install_dir comes before $(dirname "$resolved") in your PATH"
    fi
fi

detect_shell_rc() {
    case "$OSTYPE" in
        darwin*) echo "$HOME/.zshrc" ;;
        *)
            if [ -n "${ZSH_VERSION:-}" ] || [ "$(basename "${SHELL:-}")" = "zsh" ]; then
                echo "${ZDOTDIR:-$HOME}/.zshrc"
            elif [ -f "$HOME/.bashrc" ]; then
                echo "$HOME/.bashrc"
            else
                echo "$HOME/.profile"
            fi
            ;;
    esac
}

if [[ ":$PATH:" != *":$install_dir:"* ]]; then
    warn "$install_dir is not in your PATH"
    shell_rc=$(detect_shell_rc)
    echo "  Add it to your $shell_rc:"
    echo "    echo 'export PATH=\"$install_dir:\$PATH\"' >> $shell_rc"
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
        if ! confirm "  Overwrite? [y/N] " N; then
            echo "  Keeping existing config."
            write_config=0
        fi
    fi
fi

if [ "$write_config" = "1" ] && [ "$auto_yes" = "1" ]; then
    warn "Skipping credential setup in --yes mode."
    echo "  Run install.sh again without --yes to configure, or edit $config_file manually."
    write_config=0
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

if confirm "  Generate shell completions? [Y/n] " Y; then
    echo "  Shells: bash, zsh, fish"
    ask "  Shell [bash]: " "bash" shell_choice

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
else
    echo "  Skipped."
fi

# --- Authenticate ---

if [ "$upgrading" = "1" ]; then
    step "Authentication"
    echo "  Existing auth preserved. Skipping."
else
    step "Authentication"

    if [ "$auto_yes" = "0" ] && confirm "  Authenticate with Spotify now? [Y/n] " Y; then
        echo "  Running: cue devices"
        echo "  Your browser will open for Spotify authorization."
        echo
        "$cue_bin" devices || warn "Authentication did not complete. Run 'cue devices' later to retry."
    else
        echo "  Skipped. Run any cue command later to trigger authentication."
    fi
fi

# --- Done ---

if [ "$upgrading" = "1" ]; then
    step "Upgrade complete"
else
    step "Setup complete"
fi

echo "  $("$cue_bin" --version 2>/dev/null || echo "cue installed")"
echo
echo "  Quick start:"
echo "    cue devices          List available devices"
echo "    cue play <query>     Play a track"
echo "    cue now              Show what's playing"
echo "    cue --help           See all commands"
echo

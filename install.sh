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

# --- Terminal styling ---

green="" red="" yellow="" bold="" dim="" reset=""
spinner_chars='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
if [ -t 1 ] && command -v tput &>/dev/null; then
    green=$(tput setaf 2)
    red=$(tput setaf 1)
    yellow=$(tput setaf 3)
    bold=$(tput bold)
    dim=$(tput dim 2>/dev/null || true)
    reset=$(tput sgr0)
fi

ok()     { echo "  ${green}✓${reset} $1"; }
err()    { echo "  ${red}✗${reset} $1" >&2; }
warn()   { echo "  ${yellow}!${reset} $1"; }
step()   { echo; echo "${bold}$1${reset}"; }
detail() { echo "    ${dim}$1${reset}"; }

fail() { err "$1"; exit 1; }

spin() {
    local pid=$1 msg=$2
    local i=0 len=${#spinner_chars}
    while kill -0 "$pid" 2>/dev/null; do
        printf "\r  %s %s" "${spinner_chars:i%len:1}" "$msg"
        i=$((i + 1))
        sleep 0.08
    done
    printf "\r\033[2K"
}

confirm() {
    local prompt=$1 default=$2
    if [ "$auto_yes" = "1" ]; then
        [ "$default" = "Y" ]
        return
    fi
    local reply
    read -rp "  $prompt" reply
    case "${reply:-$default}" in
        [Yy]*) return 0 ;;
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

step "Checking prerequisites..."

if ! command -v cargo &>/dev/null; then
    warn "Rust is not installed"
    if confirm "Install Rust via rustup? [Y/n] " Y; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
    else
        fail "cargo is required to build cue"
    fi
fi

ok "cargo $(cargo --version | awk '{print $2}')"

# --- Build ---

if [ "$upgrading" = "1" ]; then
    step "Building cue (current: $existing_version)..."
else
    step "Building cue..."
fi

if [ -t 1 ]; then
    build_log=$(mktemp)
    cargo build --release &>"$build_log" &
    spin $! "Compiling..."
    if ! wait $!; then
        echo
        cat "$build_log" >&2
        rm -f "$build_log"
        fail "Build failed"
    fi
    rm -f "$build_log"
else
    cargo build --release || fail "Build failed"
fi

binary="$PWD/target/release/cue"
if ! "$binary" --version &>/dev/null; then
    fail "Built binary failed to run"
fi

new_version=$("$binary" --version 2>/dev/null || echo "cue")
ok "Built $new_version"

# --- Install binary ---

step "Installing binary..."

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"

if [ "$upgrading" = "1" ]; then
    install_dir=$(dirname "$existing_path")
elif [ -d "$cargo_bin" ] && [[ ":$PATH:" == *":$cargo_bin:"* ]]; then
    install_dir="$cargo_bin"
else
    install_dir="$HOME/.local/bin"
fi

mkdir -p "$install_dir"
tmpbin=$(mktemp "$install_dir/cue.XXXXXX")
cleanup="$tmpbin"
cp "$binary" "$tmpbin"
chmod 755 "$tmpbin"
mv "$tmpbin" "$install_dir/cue"
cleanup=""
ok "Installed to $install_dir/cue"

if command -v cue &>/dev/null; then
    resolved=$(command -v cue)
    if [ "$resolved" != "$install_dir/cue" ]; then
        warn "Another 'cue' at $resolved takes precedence in PATH"
        detail "Ensure $install_dir comes before $(dirname "$resolved") in PATH"
    fi
elif [[ ":$PATH:" != *":$install_dir:"* ]]; then
    warn "$install_dir is not in your PATH"
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
    shell_rc=$(detect_shell_rc)
    detail "Add to $shell_rc:"
    detail "  export PATH=\"$install_dir:\$PATH\""
fi

# --- Spotify credentials ---

step "Spotify app setup..."

case "$OSTYPE" in
    darwin*) config_dir="$HOME/Library/Application Support/cue" ;;
    *)       config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cue" ;;
esac

config_file="$config_dir/config.toml"
write_config=1

if [ -f "$config_file" ]; then
    ok "Config exists at $config_file"
    if [ "$upgrading" = "1" ]; then
        write_config=0
    elif ! confirm "Overwrite? [y/N] " N; then
        write_config=0
    fi
fi

if [ "$write_config" = "1" ] && [ "$auto_yes" = "1" ]; then
    warn "Skipping credential setup in --yes mode"
    detail "Run install.sh again or edit $config_file"
    write_config=0
fi

if [ "$write_config" = "1" ]; then
    echo
    echo "  Create a Spotify app to get your credentials:"
    echo "    1. Go to ${bold}https://developer.spotify.com/dashboard${reset}"
    echo "    2. Create a new app"
    echo "    3. Set redirect URI to ${bold}http://127.0.0.1:8888/callback${reset}"
    echo "    4. Copy the Client ID and Client Secret"
    echo

    read -rp "  Client ID: " client_id
    [ -z "$client_id" ] && fail "Client ID cannot be empty"
    [[ ! "$client_id" =~ ^[a-zA-Z0-9]+$ ]] && fail "Client ID should be alphanumeric"

    read -rsp "  Client Secret: " client_secret
    echo
    [ -z "$client_secret" ] && fail "Client Secret cannot be empty"
    [[ ! "$client_secret" =~ ^[a-zA-Z0-9]+$ ]] && fail "Client Secret should be alphanumeric"

    mkdir -p "$config_dir"
    (
        umask 077
        cat > "$config_file" <<EOF
[spotify]
client_id = "$client_id"
client_secret = "$client_secret"
EOF
    )
    ok "Config written to $config_file"
fi

# --- Shell completions ---

step "Setting up shell completions..."

cue_bin="$install_dir/cue"
shell_name=$(basename "${SHELL:-bash}")

case "$shell_name" in
    bash)
        comp_file="$HOME/.local/share/bash-completion/completions/cue"
        mkdir -p "${comp_file%/*}"
        "$cue_bin" completions bash > "$comp_file"
        ok "bash completions installed"
        detail "$comp_file"
        ;;
    zsh)
        comp_dir="${ZDOTDIR:-$HOME}/.zfunc"
        mkdir -p "$comp_dir"
        "$cue_bin" completions zsh > "$comp_dir/_cue"
        ok "zsh completions installed"
        detail "$comp_dir/_cue"
        ;;
    fish)
        comp_dir="$HOME/.config/fish/completions"
        mkdir -p "$comp_dir"
        "$cue_bin" completions fish > "$comp_dir/cue.fish"
        ok "fish completions installed"
        detail "$comp_dir/cue.fish"
        ;;
    *)
        warn "Unknown shell '$shell_name', skipping completions"
        detail "Run 'cue completions --help' to install manually"
        ;;
esac

# --- Authenticate ---

if [ "$upgrading" = "1" ]; then
    step "Authentication..."
    ok "Existing auth preserved"
elif [ "$auto_yes" = "0" ]; then
    step "Authenticating with Spotify..."
    echo "  Your browser will open for authorization."
    echo
    "$cue_bin" devices || warn "Authentication did not complete. Run 'cue devices' to retry."
fi

# --- Done ---

echo
echo "${green}${bold}✓${reset}${bold} $new_version installed successfully${reset}"
echo
echo "  Quick start:"
echo "    ${bold}cue devices${reset}          List available devices"
echo "    ${bold}cue play <query>${reset}     Play a track"
echo "    ${bold}cue now${reset}              Show what's playing"
echo "    ${bold}cue --help${reset}           See all commands"
echo

#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_NAME="filmcraft"
APP_NAME="FilmCraft"
INSTALL_PREFIX="${APP_INSTALL_PREFIX:-$HOME/.local}"
APPS_LIST="${APPS_LIST:-${XDG_CONFIG_HOME:-$HOME/.config}/apps.list}"
INSTALL_TARGET=install
UPDATE=0
DESKTOP_ONLY=0
DRY_RUN=0

usage() {
    printf '%s\n' \
        'Usage: build-install.sh [--dry-run] [--update | --desktop-only]' \
        "Build and install $APP_NAME and its CLI, then register its launcher in apps.list." \
        '--desktop-only repairs launchers and icons without building or pulling.' \
        'Configuration: APP_INSTALL_PREFIX (default: ~/.local), APPS_LIST.' \
        'Makefile settings may be set in the environment: JOBS, BUILD_PROFILE, FEATURES, CARGO_FLAGS.'
}

parse_arguments() {
    while (($#)); do
        case "$1" in
            --dry-run) DRY_RUN=1 ;;
            --update) UPDATE=1 ;;
            --desktop-only) DESKTOP_ONLY=1 ;;
            -h|--help) usage; exit 0 ;;
            *) printf 'Unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
        esac
        shift
    done
    if [[ "$INSTALL_PREFIX" != /* || "$APPS_LIST" != /* ]]; then
        printf 'APP_INSTALL_PREFIX and APPS_LIST must be absolute paths.\n' >&2
        exit 2
    fi
    if ((UPDATE && DESKTOP_ONLY)); then
        printf '%s\n' '--update and --desktop-only cannot be combined.' >&2
        exit 2
    fi
    if ((UPDATE)); then
        INSTALL_TARGET=update
    elif ((DESKTOP_ONLY)); then
        INSTALL_TARGET=install-desktop
    fi
}

install_project() {
    local -a make_arguments=(-C "$PROJECT_ROOT" "$INSTALL_TARGET" "PREFIX=$INSTALL_PREFIX")
    if [[ ! -f "$PROJECT_ROOT/Makefile" ]]; then
        printf 'Missing Makefile: %s/Makefile\n' "$PROJECT_ROOT" >&2
        return 1
    fi
    if ((DRY_RUN)); then
        printf '%q ' /usr/bin/make "${make_arguments[@]}"
        printf '\n'
        return
    fi
    /usr/bin/make "${make_arguments[@]}"
}

register_app() {
    local project="$1"
    local app_name="$2"
    local desktop_id="ai.storyteller.$project.desktop"
    local icon_path="$INSTALL_PREFIX/share/icons/hicolor/256x256/apps/ai.storyteller.$project.png"
    local entry="$desktop_id|$icon_path|$app_name"

    if ((!DRY_RUN)) && [[ ! -x "$INSTALL_PREFIX/bin/$project" || ! -f "$icon_path" || \
        ! -f "$INSTALL_PREFIX/share/applications/$desktop_id" ]]; then
        printf 'Installation is incomplete for %s.\n' "$app_name" >&2
        return 1
    fi
    if [[ -f "$APPS_LIST" ]] && /usr/bin/awk -F '|' -v desktop_id="$desktop_id" \
        '$1 == desktop_id { found = 1 } END { exit !found }' "$APPS_LIST"; then
        return
    fi
    if ((DRY_RUN)); then
        printf 'Append to %s: %s\n' "$APPS_LIST" "$entry"
        return
    fi
    /usr/bin/mkdir -p -- "$(/usr/bin/dirname -- "$APPS_LIST")"
    if [[ -s "$APPS_LIST" && -n "$(/usr/bin/tail -c 1 -- "$APPS_LIST")" ]]; then
        printf '\n' >> "$APPS_LIST"
    fi
    printf '%s\n' "$entry" >> "$APPS_LIST"
}

main() {
    parse_arguments "$@"
    install_project
    register_app "$PROJECT_NAME" "$APP_NAME"
}

main "$@"

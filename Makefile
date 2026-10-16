SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := install

CARGO ?= cargo
DESKTOP_FILE_INSTALL ?= desktop-file-install
CARGO_FLAGS ?=
FEATURES ?=
JOBS ?= 4
CARGO_TARGET_DIR ?= target
BUILD_PROFILE ?= release
PROFILE_DIR ?= $(if $(filter dev,$(BUILD_PROFILE)),debug,$(BUILD_PROFILE))
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=

DESKTOP_ID := ai.storyteller.filmcraft
ICON_ROOT := assets/app-icon/hicolor

export CARGO_TARGET_DIR

.PHONY: all build install install-desktop update update-install help

all: install

build:
	"$(CARGO)" build --locked --profile "$(BUILD_PROFILE)" --jobs "$(JOBS)" \
		--target-dir "$(CARGO_TARGET_DIR)" -p filmcraft -p filmcraft-cli --bins \
		$(if $(strip $(FEATURES)),--features "$(FEATURES)") $(CARGO_FLAGS)

install: build
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 755 "$(CARGO_TARGET_DIR)/$(PROFILE_DIR)/filmcraft" "$(DESTDIR)$(BINDIR)/filmcraft"
	install -m 755 "$(CARGO_TARGET_DIR)/$(PROFILE_DIR)/filmcraft-cli" "$(DESTDIR)$(BINDIR)/filmcraft-cli"
	$(MAKE) --no-print-directory install-desktop

install-desktop:
	test -x "$(DESTDIR)$(BINDIR)/filmcraft"
	"$(DESKTOP_FILE_INSTALL)" --dir="$(DESTDIR)$(DATADIR)/applications" \
		--mode=644 --set-key=Exec --set-value='"$(BINDIR)/filmcraft" %F' \
		--set-key=TryExec --set-value="$(BINDIR)/filmcraft" \
		--rebuild-mime-info-cache "packaging/linux/$(DESKTOP_ID).desktop"
	for icon in "$(ICON_ROOT)"/*/apps/$(DESKTOP_ID).png "$(ICON_ROOT)"/scalable/apps/$(DESKTOP_ID).svg; do \
		[ -f "$$icon" ] || continue; \
		install -D -m 644 "$$icon" "$(DESTDIR)$(DATADIR)/icons/hicolor/$${icon#$(ICON_ROOT)/}"; \
	done

update:
	git pull --ff-only
	$(MAKE) --no-print-directory install

update-install: update

help:
	@printf '%s\n' \
	  'make [install]       Build and install FilmCraft and filmcraft-cli' \
	  'make build           Build both native binaries' \
	  'make install-desktop Repair launchers and icons without rebuilding' \
	  'make update          Fast-forward from GitHub, then build and install' \
	  'make help            Show this help' \
	  '' \
	  'Config: PREFIX, BINDIR, DATADIR, DESTDIR, BUILD_PROFILE, PROFILE_DIR,' \
	  '        CARGO, CARGO_FLAGS, CARGO_TARGET_DIR, FEATURES, JOBS, DESKTOP_FILE_INSTALL' \
	  'Desktop installation requires desktop-file-utils.' \
	  'Preview without changes: make -n update'

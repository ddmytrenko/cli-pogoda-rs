# pogoda — convenience build/install targets.
#
#   make            build the release binary
#   make test       run the test suite
#   make install    install the binary (via cargo, into ~/.cargo/bin) and the man page
#   make uninstall  remove both
#   make purge      uninstall, then delete the config and cache dirs (destructive)
#   make clean      cargo clean
#
# The man page is installed under $(MANDIR). Override MANPREFIX/MANDIR to
# relocate it, e.g.  sudo make install MANPREFIX=/usr/local

CARGO     ?= cargo
INSTALL   ?= install
PKG       := cli-pogoda-rs
BIN       := pogoda
APP       := pogoda-rs
MANPAGE   := man/pogoda.1
MANPREFIX ?= $(HOME)/.local
MANDIR    ?= $(MANPREFIX)/share/man/man1

# User data dirs, following the same XDG rules as the app (empty var -> fallback).
CONFIGDIR ?= $(or $(XDG_CONFIG_HOME),$(HOME)/.config)/$(APP)
CACHEDIR  ?= $(or $(XDG_CACHE_HOME),$(HOME)/.cache)/$(APP)

.PHONY: all build test install uninstall purge clean

all: build

build:
	$(CARGO) build --release

test:
	$(CARGO) test

install:
	$(CARGO) install --path . --force
	$(INSTALL) -d "$(MANDIR)"
	$(INSTALL) -m 0644 $(MANPAGE) "$(MANDIR)/$(BIN).1"
	@echo "Installed $(BIN) (~/.cargo/bin) and man page ($(MANDIR)/$(BIN).1)."

uninstall:
	-$(CARGO) uninstall $(PKG)
	-rm -f "$(MANDIR)/$(BIN).1"
	@echo "Removed $(BIN) and its man page."

purge: uninstall
	-rm -rf "$(CONFIGDIR)" "$(CACHEDIR)"
	@echo "Purged config ($(CONFIGDIR)) and cache ($(CACHEDIR))."

clean:
	$(CARGO) clean

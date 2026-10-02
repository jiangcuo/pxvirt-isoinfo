PACKAGE = pxvirt-isoinfo
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
PKG_RELEASE = 1

BUILDDIR ?= $(CURDIR)/build
CARGO ?= cargo
# e.g. x86_64-unknown-linux-musl for a static binary
RUST_TARGET ?=

ifeq ($(RUST_TARGET),)
CARGO_OUT = target/release
else
CARGO_OUT = target/$(RUST_TARGET)/release
CARGO_FLAGS += --target $(RUST_TARGET)
endif

BIN = $(BUILDDIR)/$(PACKAGE)
RPMTOP = $(BUILDDIR)/rpmbuild

.PHONY: all
all: $(BIN)

$(BIN): Cargo.toml Cargo.lock rules.json $(wildcard src/*.rs)
	$(CARGO) build --release --locked $(CARGO_FLAGS)
	mkdir -p $(BUILDDIR)
	cp $(CARGO_OUT)/$(PACKAGE) $@

.PHONY: check
check:
	$(CARGO) test --locked

.PHONY: install
install: $(BIN)
	install -D -m 0755 $(BIN) $(DESTDIR)/usr/bin/$(PACKAGE)
	install -D -m 0644 rules.json $(DESTDIR)/usr/share/$(PACKAGE)/rules.json
	install -d $(DESTDIR)/etc/$(PACKAGE)/rules.d

.PHONY: deb
deb: $(BIN)
	test "$$(dpkg-parsechangelog -S Version)" = "$(VERSION)-$(PKG_RELEASE)" \
	    || (echo "debian/changelog does not match Cargo.toml" && false)
	rm -rf debian/$(PACKAGE) debian/.debhelper debian/files debian/*.substvars
	dpkg-buildpackage -b -us -uc --no-pre-clean
	mkdir -p $(BUILDDIR)/out
	mv ../$(PACKAGE)_$(VERSION)-$(PKG_RELEASE)_*.deb $(BUILDDIR)/out/
	rm -f ../$(PACKAGE)_*.buildinfo ../$(PACKAGE)_*.changes

.PHONY: rpm
rpm: $(BIN)
	mkdir -p $(RPMTOP)/SOURCES $(BUILDDIR)/out
	cp $(BIN) rules.json $(RPMTOP)/SOURCES/
	rpmbuild -bb \
	    --define "_topdir $(RPMTOP)" \
	    --define "pkgversion $(VERSION)" \
	    --define "pkgrelease $(PKG_RELEASE)" \
	    rpm/$(PACKAGE).spec
	find $(RPMTOP)/RPMS -name '*.rpm' -exec mv {} $(BUILDDIR)/out/ \;

.PHONY: clean
clean:
	rm -rf $(BUILDDIR) target debian/$(PACKAGE) debian/.debhelper debian/files \
	    debian/*.substvars debian/*.debhelper.log debian/debhelper-build-stamp

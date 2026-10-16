APP_NAME   := Asides
BUNDLE_ID  := io.github.mihaicristianfarcas.asides
VERSION    := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP        := dist/$(APP_NAME).app
# The CLI ships inside the app, so one signed (and notarized) bundle carries both. So does the
# Measure & Annotate overlay (GPUI, its own process), beside the app's binary.
CLI_IN_APP := Contents/Helpers/asides
OVERLAY_IN_APP := Contents/MacOS/asides-overlay
APPS_DIR   ?= $(HOME)/Applications
BIN_DIR    ?= $(HOME)/.local/bin
# A stable signing identity keeps the microphone permission across rebuilds. Picked automatically:
# Developer ID Application, Apple Development, "Asides Dev" (`make cert`), else ad hoc ("-").
SIGN_ID    ?= $(shell scripts/signing-identity.sh)
# Local builds skip the secure timestamp (no network needed); `make release` asks for it.
TIMESTAMP  ?= --timestamp=none
# Hardened runtime everywhere; the app's entitlements allow the microphone.
CODESIGN    = codesign --force --options runtime $(TIMESTAMP) --sign "$(SIGN_ID)"
FEATURES   ?=
LSREGISTER := /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
CARGO_FEATURES := $(if $(FEATURES),--features asides-app/$(FEATURES))

FIXTURES   := tests/fixtures/en_bug.wav tests/fixtures/ro_bug.wav tests/fixtures/screenshot.png

.PHONY: build test test-speech bundle install uninstall release publish-dmg publish-site e2e cert fixtures clean

build:
	cargo build --release --workspace $(CARGO_FEATURES)

test:
	cargo test --workspace

# Transcription and model-download tests; need the speech model (and network for the download test).
test-speech: fixtures
	cargo test --release -p asides-app -- --ignored --nocapture

bundle: build
	rm -rf $(APP)
	mkdir -p $(APP)/Contents/MacOS $(APP)/Contents/Helpers $(APP)/Contents/Resources
	sed 's/__VERSION__/$(VERSION)/g' packaging/Info.plist > $(APP)/Contents/Info.plist
	cp target/release/asides-app $(APP)/Contents/MacOS/$(APP_NAME)
	cp target/release/asides $(APP)/$(CLI_IN_APP)
	cp target/release/asides-overlay $(APP)/$(OVERLAY_IN_APP)
	cp packaging/Asides.icns $(APP)/Contents/Resources/
	# Sign inside-out: the nested CLI and overlay first, then the app.
	$(CODESIGN) --identifier $(BUNDLE_ID).cli $(APP)/$(CLI_IN_APP)
	$(CODESIGN) --identifier $(BUNDLE_ID).overlay $(APP)/$(OVERLAY_IN_APP)
	$(CODESIGN) --entitlements packaging/Asides.entitlements $(APP)
	@echo "Bundled $(APP) (signed: $(SIGN_ID))"

install: bundle
	-@test -x $(BIN_DIR)/asides && $(BIN_DIR)/asides quit >/dev/null 2>&1; sleep 1
	-@pkill -x $(APP_NAME) 2>/dev/null; true
	mkdir -p $(APPS_DIR) $(BIN_DIR)
	rm -rf $(APPS_DIR)/$(APP_NAME).app
	ditto $(APP) $(APPS_DIR)/$(APP_NAME).app
	@$(LSREGISTER) -u $(CURDIR)/$(APP) 2>/dev/null || true
	$(LSREGISTER) -f $(APPS_DIR)/$(APP_NAME).app
	ln -sfn $(APPS_DIR)/$(APP_NAME).app/$(CLI_IN_APP) $(BIN_DIR)/asides
	@echo "Installed $(APPS_DIR)/$(APP_NAME).app; $(BIN_DIR)/asides links to its CLI"

uninstall:
	-@test -x $(BIN_DIR)/asides && $(BIN_DIR)/asides quit >/dev/null 2>&1; sleep 1
	-@pkill -x $(APP_NAME) 2>/dev/null; true
	-$(LSREGISTER) -u $(APPS_DIR)/$(APP_NAME).app
	rm -rf $(APPS_DIR)/$(APP_NAME).app
	rm -f $(BIN_DIR)/asides
	@echo "Removed. Settings and models remain in ~/Library/Application Support/$(APP_NAME)."

# Developer ID signature with secure timestamp → Apple notarization → stapled
# dist/Asides-<version>.zip. Needs a notarytool profile (NOTARY_PROFILE, default asides-notary).
release:
	scripts/release.sh

# Deploys dist/Asides-<version>.dmg (from `make release`) behind the website's download button,
# on Cloudflare. Needs Wrangler, logged in.
publish-dmg:
	scripts/publish-dmg.sh

# Deploys the website (docs/) to asides.mihaicristian.dev, on Cloudflare. Needs Wrangler, logged in.
publish-site:
	wrangler deploy --config packaging/site/wrangler.jsonc

# End-to-end test against an installed build with the dev hooks (feeds WAV fixtures instead of the mic).
e2e: fixtures
	$(MAKE) install FEATURES=dev-hooks
	scripts/e2e.sh

# Without an Apple certificate: creates a self-signed "Asides Dev" identity (asks for your password once).
cert:
	scripts/make-signing-cert.sh

fixtures: $(FIXTURES)

$(FIXTURES):
	scripts/make-fixtures.sh

clean:
	cargo clean
	rm -rf dist

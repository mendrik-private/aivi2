.PHONY: build install run clean

MANIFEST := org.aivi.Aivi.yml
BUILD_DIR := .flatpak-build
REPO := repo
APP_ID := org.aivi.Aivi

build:
	flatpak-builder --user --install-deps-from=flathub --repo=$(REPO) $(BUILD_DIR) $(MANIFEST)

install: build
	flatpak-builder --user --install-deps-from=flathub --repo=$(REPO) $(BUILD_DIR) $(MANIFEST)

run: install
	flatpak run $(APP_ID)

clean:
	rm -rf $(BUILD_DIR) $(REPO) .flatpak-builder

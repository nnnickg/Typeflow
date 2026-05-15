ROOT_DIR := $(abspath ..)
CARGO_TARGET_DIR ?= $(ROOT_DIR)/target
RUST_PROFILE ?= release
SWIFT ?= xcrun swift
SWIFTC ?= xcrun swiftc
CODESIGN ?= codesign
CODESIGN_IDENTITY ?= -
CODESIGN_FLAGS ?=
APPLICATIONS_DIR ?= $(HOME)/Applications

ifeq ($(RUST_PROFILE),release)
CARGO_PROFILE_FLAG := --release
RUST_LIB_DIR := $(CARGO_TARGET_DIR)/release
SWIFT_OPT_FLAGS := -O -whole-module-optimization
SWIFT_PACKAGE_CONFIG := release
else
CARGO_PROFILE_FLAG :=
RUST_LIB_DIR := $(CARGO_TARGET_DIR)/debug
SWIFT_OPT_FLAGS := -Onone
SWIFT_PACKAGE_CONFIG := debug
endif

ifeq ($(CODESIGN_IDENTITY),-)
CODESIGN_RUNTIME_FLAGS :=
else
CODESIGN_RUNTIME_FLAGS := --options runtime --timestamp
endif

FFI_INCLUDE_DIR := $(abspath TypeflowFFI/include)
RUST_STATICLIB := $(RUST_LIB_DIR)/libtypeflow_ffi.a
BUILD_DIR := $(abspath build)
MODULE_CACHE_DIR := $(BUILD_DIR)/clang-module-cache
SMOKE_BIN := $(BUILD_DIR)/typeflow-staticlib-smoke
APP_NAME := Typeflow
APP_BUNDLE := $(BUILD_DIR)/$(APP_NAME).app
APP_EXECUTABLE := $(APP_BUNDLE)/Contents/MacOS/$(APP_NAME)
APP_INFO_PLIST := Resources/Info.plist
APP_PKGINFO := Resources/PkgInfo
APP_INFO_PLIST_STRINGS := Resources/en.lproj/InfoPlist.strings
ICON_SOURCE := Resources/Typeflow.png
ICON_TIFF_DIR := $(BUILD_DIR)/$(APP_NAME)-icon-tiffs
ICON_MULTI_TIFF := $(BUILD_DIR)/$(APP_NAME)-icons.tiff
APP_ICON_ICNS := $(APP_BUNDLE)/Contents/Resources/Typeflow.icns
APP_ICONS := $(APP_ICON_ICNS)
KIT_SOURCES := $(wildcard Sources/TypeflowKit/*.swift)
SMOKE_SOURCES := $(KIT_SOURCES) $(wildcard Sources/TypeflowSmoke/*.swift)
AGENT_SOURCES := $(KIT_SOURCES) $(wildcard Sources/TypeflowAgent/*.swift)
SWIFT_COMMON_FLAGS := $(SWIFT_OPT_FLAGS) -module-cache-path $(MODULE_CACHE_DIR) -I $(FFI_INCLUDE_DIR)
RUST_LINK_FLAGS := -Xlinker -force_load -Xlinker $(RUST_STATICLIB)
AGENT_FRAMEWORK_FLAGS := -framework AppKit -framework ApplicationServices -framework Carbon -framework ServiceManagement -framework IOKit

.PHONY: all rust-staticlib smoke swift-package bundle release-universal install-user clean

all: smoke bundle

rust-staticlib:
	cd $(ROOT_DIR) && CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build $(CARGO_PROFILE_FLAG) -p typeflow-ffi

$(SMOKE_BIN): $(SMOKE_SOURCES) TypeflowFFI/include/module.modulemap TypeflowFFI/include/typeflow_shim.h rust-staticlib
	mkdir -p $(BUILD_DIR) $(MODULE_CACHE_DIR)
	$(SWIFTC) $(SWIFT_COMMON_FLAGS) $(SMOKE_SOURCES) $(RUST_LINK_FLAGS) -o $(SMOKE_BIN)

smoke: $(SMOKE_BIN)
	$(SMOKE_BIN)

swift-package: rust-staticlib
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) RUST_PROFILE=$(RUST_PROFILE) $(SWIFT) run --package-path . -c $(SWIFT_PACKAGE_CONFIG) typeflow-staticlib-smoke
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) RUST_PROFILE=$(RUST_PROFILE) $(SWIFT) build --package-path . -c $(SWIFT_PACKAGE_CONFIG) --product Typeflow

$(APP_ICON_ICNS): $(ICON_SOURCE)
	mkdir -p $(APP_BUNDLE)/Contents/Resources $(ICON_TIFF_DIR)
	sips -s format tiff --resampleHeightWidth 16 16 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_16.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 32 32 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_32.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 48 48 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_48.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 128 128 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_128.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 256 256 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_256.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 512 512 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_512.tiff >/dev/null
	sips -s format tiff --resampleHeightWidth 1024 1024 $(ICON_SOURCE) --out $(ICON_TIFF_DIR)/icon_1024.tiff >/dev/null
	tiffutil -cat $(ICON_TIFF_DIR)/icon_16.tiff $(ICON_TIFF_DIR)/icon_32.tiff $(ICON_TIFF_DIR)/icon_48.tiff $(ICON_TIFF_DIR)/icon_128.tiff $(ICON_TIFF_DIR)/icon_256.tiff $(ICON_TIFF_DIR)/icon_512.tiff $(ICON_TIFF_DIR)/icon_1024.tiff -out $(ICON_MULTI_TIFF) >/dev/null 2>/dev/null
	tiff2icns $(ICON_MULTI_TIFF) $(APP_ICON_ICNS)

$(APP_EXECUTABLE): $(AGENT_SOURCES) $(APP_INFO_PLIST) $(APP_INFO_PLIST_STRINGS) $(APP_PKGINFO) $(APP_ICONS) TypeflowFFI/include/module.modulemap TypeflowFFI/include/typeflow_shim.h rust-staticlib
	mkdir -p $(APP_BUNDLE)/Contents/MacOS $(APP_BUNDLE)/Contents/Resources/en.lproj $(MODULE_CACHE_DIR)
	cp $(APP_INFO_PLIST) $(APP_BUNDLE)/Contents/Info.plist
	cp $(APP_INFO_PLIST_STRINGS) $(APP_BUNDLE)/Contents/Resources/en.lproj/InfoPlist.strings
	cp $(APP_PKGINFO) $(APP_BUNDLE)/Contents/PkgInfo
	plutil -lint $(APP_BUNDLE)/Contents/Info.plist
	$(SWIFTC) $(SWIFT_COMMON_FLAGS) $(AGENT_SOURCES) $(RUST_LINK_FLAGS) $(AGENT_FRAMEWORK_FLAGS) -o $(APP_EXECUTABLE)

bundle: $(APP_EXECUTABLE)
	plutil -p $(APP_BUNDLE)/Contents/Info.plist >/dev/null
	$(CODESIGN) --force --sign "$(CODESIGN_IDENTITY)" $(CODESIGN_RUNTIME_FLAGS) $(CODESIGN_FLAGS) $(APP_BUNDLE)
	$(CODESIGN) --verify --strict --verbose=2 $(APP_BUNDLE)
	file $(APP_EXECUTABLE)

release-universal:
	./scripts/package-universal.sh

install-user: bundle
	mkdir -p "$(APPLICATIONS_DIR)"
	ditto "$(APP_BUNDLE)" "$(APPLICATIONS_DIR)/$(APP_NAME).app"
	xattr -cr "$(APPLICATIONS_DIR)/$(APP_NAME).app" || true
	open -g "$(APPLICATIONS_DIR)/$(APP_NAME).app"

clean:
	rm -rf $(BUILD_DIR)

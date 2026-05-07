ROOT_DIR := $(abspath ..)
CARGO_TARGET_DIR ?= $(ROOT_DIR)/target
RUST_PROFILE ?= release
SWIFTC ?= xcrun swiftc
CODESIGN ?= codesign
CODESIGN_IDENTITY ?= -
INPUT_METHODS_DIR ?= $(HOME)/Library/Input Methods

ifeq ($(RUST_PROFILE),release)
CARGO_PROFILE_FLAG := --release
RUST_LIB_DIR := $(CARGO_TARGET_DIR)/release
SWIFT_OPT_FLAGS := -O -whole-module-optimization
else
CARGO_PROFILE_FLAG :=
RUST_LIB_DIR := $(CARGO_TARGET_DIR)/debug
SWIFT_OPT_FLAGS := -Onone
endif

FFI_INCLUDE_DIR := $(abspath TypeflowFFI/include)
RUST_STATICLIB := $(RUST_LIB_DIR)/libtypeflow_ffi.a
BUILD_DIR := $(abspath build)
MODULE_CACHE_DIR := $(BUILD_DIR)/clang-module-cache
SMOKE_BIN := $(BUILD_DIR)/typeflow-staticlib-smoke
REGISTER_BIN := $(BUILD_DIR)/typeflow-register-input-source
APP_NAME := Typeflow
APP_BUNDLE := $(BUILD_DIR)/$(APP_NAME).app
APP_EXECUTABLE := $(APP_BUNDLE)/Contents/MacOS/$(APP_NAME)
APP_INFO_PLIST := Resources/Info.plist
APP_PKGINFO := Resources/PkgInfo
ICON_SOURCE := Resources/Typeflow.png
ICON_TIFF_DIR := $(BUILD_DIR)/$(APP_NAME)-icon-tiffs
ICON_MULTI_TIFF := $(BUILD_DIR)/$(APP_NAME)-icons.tiff
APP_ICON_ICNS := $(APP_BUNDLE)/Contents/Resources/Typeflow.icns
APP_ICON_TIFF := $(APP_BUNDLE)/Contents/Resources/Typeflow.tiff
APP_ICONS := $(APP_ICON_ICNS) $(APP_ICON_TIFF)
KIT_SOURCES := $(wildcard Sources/TypeflowKit/*.swift)
SMOKE_SOURCES := $(KIT_SOURCES) $(wildcard Sources/TypeflowSmoke/*.swift)
INPUT_METHOD_SOURCES := $(KIT_SOURCES) $(wildcard Sources/TypeflowInputMethod/*.swift)
REGISTER_SOURCES := $(wildcard Sources/TypeflowRegister/*.swift)
SWIFT_COMMON_FLAGS := $(SWIFT_OPT_FLAGS) -module-cache-path $(MODULE_CACHE_DIR) -I $(FFI_INCLUDE_DIR)
RUST_LINK_FLAGS := -Xlinker -force_load -Xlinker $(RUST_STATICLIB)
IMK_FRAMEWORK_FLAGS := -framework AppKit -framework Carbon -framework InputMethodKit

.PHONY: all rust-staticlib smoke bundle install-user clean

all: smoke bundle

rust-staticlib:
	cd $(ROOT_DIR) && CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build $(CARGO_PROFILE_FLAG) -p typeflow-ffi

$(SMOKE_BIN): $(SMOKE_SOURCES) TypeflowFFI/include/module.modulemap TypeflowFFI/include/typeflow_shim.h rust-staticlib
	mkdir -p $(BUILD_DIR) $(MODULE_CACHE_DIR)
	$(SWIFTC) $(SWIFT_COMMON_FLAGS) $(SMOKE_SOURCES) $(RUST_LINK_FLAGS) -o $(SMOKE_BIN)

smoke: $(SMOKE_BIN)
	$(SMOKE_BIN)

$(REGISTER_BIN): $(REGISTER_SOURCES)
	mkdir -p $(BUILD_DIR) $(MODULE_CACHE_DIR)
	$(SWIFTC) $(SWIFT_OPT_FLAGS) -module-cache-path $(MODULE_CACHE_DIR) $(REGISTER_SOURCES) -framework Carbon -o $(REGISTER_BIN)

$(APP_ICON_TIFF): $(ICON_SOURCE)
	mkdir -p $(APP_BUNDLE)/Contents/Resources
	sips -s format tiff --resampleHeightWidth 32 32 $(ICON_SOURCE) --out $(APP_ICON_TIFF) >/dev/null

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

$(APP_EXECUTABLE): $(INPUT_METHOD_SOURCES) $(APP_INFO_PLIST) $(APP_PKGINFO) $(APP_ICONS) TypeflowFFI/include/module.modulemap TypeflowFFI/include/typeflow_shim.h rust-staticlib
	mkdir -p $(APP_BUNDLE)/Contents/MacOS $(APP_BUNDLE)/Contents/Resources $(MODULE_CACHE_DIR)
	cp $(APP_INFO_PLIST) $(APP_BUNDLE)/Contents/Info.plist
	cp $(APP_PKGINFO) $(APP_BUNDLE)/Contents/PkgInfo
	plutil -lint $(APP_BUNDLE)/Contents/Info.plist
	$(SWIFTC) $(SWIFT_COMMON_FLAGS) $(INPUT_METHOD_SOURCES) $(RUST_LINK_FLAGS) $(IMK_FRAMEWORK_FLAGS) -o $(APP_EXECUTABLE)

bundle: $(APP_EXECUTABLE) $(REGISTER_BIN)
	plutil -p $(APP_BUNDLE)/Contents/Info.plist >/dev/null
	$(CODESIGN) --force --sign $(CODESIGN_IDENTITY) $(APP_BUNDLE)
	$(CODESIGN) --verify --strict --verbose=2 $(APP_BUNDLE)
	file $(APP_EXECUTABLE)

install-user: bundle $(REGISTER_BIN)
	mkdir -p "$(INPUT_METHODS_DIR)"
	ditto "$(APP_BUNDLE)" "$(INPUT_METHODS_DIR)/$(APP_NAME).app"
	$(REGISTER_BIN) "$(INPUT_METHODS_DIR)/$(APP_NAME).app"

clean:
	rm -rf $(BUILD_DIR)

include $(TOPDIR)/rules.mk

PKG_NAME:=xboxdownload
PKG_VERSION:=0.1.0
PKG_RELEASE:=1
PKG_LICENSE:=GPL-3.0-or-later

# A symlink to this package in the SDK keeps the repository data/source accessible.
XBOXDOWNLOAD_SOURCE_DIR?=$(abspath $(realpath $(CURDIR))/../..)
RUST_TARGET_aarch64:=aarch64-unknown-linux-musl
RUST_TARGET_x86_64:=x86_64-unknown-linux-musl
RUST_TARGET_arm:=armv7-unknown-linux-musleabihf
RUST_TARGET_mipsel:=mipsel-unknown-linux-musl
XBOXDOWNLOAD_RUST_TARGET?=$(RUST_TARGET_$(ARCH))
RUST_ENV_TARGET=$(subst -,_,$(XBOXDOWNLOAD_RUST_TARGET))
RUST_LINKER_ENV=CARGO_TARGET_$(shell echo $(RUST_ENV_TARGET) | tr a-z A-Z)_LINKER
PKG_FILE_DEPENDS:=$(XBOXDOWNLOAD_SOURCE_DIR)/data
ifneq ($(strip $(XBOXDOWNLOAD_PREBUILT_BIN)),)
  PKG_FILE_DEPENDS+=$(XBOXDOWNLOAD_PREBUILT_BIN)
else
  PKG_FILE_DEPENDS+=$(addprefix $(XBOXDOWNLOAD_SOURCE_DIR)/,Cargo.toml Cargo.lock daemon)
endif

include $(INCLUDE_DIR)/package.mk

define Package/xboxdownload
  SECTION:=net
  CATEGORY:=Network
  TITLE:=Xbox and Microsoft Store download accelerator
  # The ip command is supplied by BusyBox, ip-tiny or ip-full on OpenWrt.
  # Both nftables variants provide the text CLI. Polling flock -n for a timeout
  # works with BusyBox and util-linux without a package conflict on BusyBox firmware.
  DEPENDS:=+nftables +kmod-nft-nat +ca-bundle
endef

define Package/xboxdownload/conffiles
/etc/config/xboxdownload
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) $(XBOXDOWNLOAD_SOURCE_DIR)/data $(PKG_BUILD_DIR)/
	$(if $(strip $(XBOXDOWNLOAD_PREBUILT_BIN)),,$(CP) $(addprefix $(XBOXDOWNLOAD_SOURCE_DIR)/,Cargo.toml Cargo.lock daemon) $(PKG_BUILD_DIR)/)
endef

ifneq ($(strip $(XBOXDOWNLOAD_PREBUILT_BIN)),)
  define Build/Compile
	$(INSTALL_BIN) "$(XBOXDOWNLOAD_PREBUILT_BIN)" $(PKG_BUILD_DIR)/xboxdownloadd
  endef
else
  # Supply the Rust target with rustup. Tier 3 MIPS needs nightly + rust-src.
  define Build/Compile
	@test -n "$(XBOXDOWNLOAD_RUST_TARGET)" || { echo 'Set XBOXDOWNLOAD_RUST_TARGET for this architecture'; exit 1; }
	mkdir -p $(PKG_BUILD_DIR)/rust-compat
	eh=$$($(TARGET_CROSS)gcc -print-file-name=libgcc_eh.a); \
		if [ -f "$$eh" ]; then ln -sf "$$eh" $(PKG_BUILD_DIR)/rust-compat/libunwind.a; else $(TARGET_CROSS)ar rcs $(PKG_BUILD_DIR)/rust-compat/libunwind.a; fi
	cd $(PKG_BUILD_DIR) && \
		$(RUST_LINKER_ENV)="$(TARGET_CROSS)gcc" CC_$(RUST_ENV_TARGET)="$(TARGET_CROSS)gcc" \
		AR_$(RUST_ENV_TARGET)="$(TARGET_CROSS)ar" \
		RUSTFLAGS='-C target-feature=+crt-static -C link-self-contained=no -C relocation-model=static -L native=$(PKG_BUILD_DIR)/rust-compat' \
		cargo $(if $(filter mipsel-unknown-linux-musl,$(XBOXDOWNLOAD_RUST_TARGET)),+nightly) build --release --locked \
			$(if $(filter mipsel-unknown-linux-musl,$(XBOXDOWNLOAD_RUST_TARGET)),-Zbuild-std=std$(comma)panic_abort) \
			--target $(XBOXDOWNLOAD_RUST_TARGET) --target-dir target
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/target/$(XBOXDOWNLOAD_RUST_TARGET)/release/xboxdownloadd $(PKG_BUILD_DIR)/xboxdownloadd
  endef
endif

define Package/xboxdownload/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/etc/config $(1)/etc/init.d $(1)/etc/hotplug.d/iface $(1)/etc/xboxdownload $(1)/lib/upgrade/keep.d
	$(INSTALL_DIR) $(1)/usr/libexec/xboxdownload $(1)/usr/share/xboxdownload/ip
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/xboxdownloadd $(1)/usr/bin/
	$(INSTALL_CONF) ./files/xboxdownload.config $(1)/etc/config/xboxdownload
	$(INSTALL_BIN) ./files/xboxdownload.init $(1)/etc/init.d/xboxdownload
	$(INSTALL_DATA) ./files/95-xboxdownload.hotplug $(1)/etc/hotplug.d/iface/95-xboxdownload
	printf '%s\n' /etc/xboxdownload/ > $(1)/lib/upgrade/keep.d/xboxdownload
	$(INSTALL_BIN) ./files/xboxdownload-apply-ip $(1)/usr/libexec/xboxdownload/apply-ip
	$(INSTALL_DATA) $(addprefix $(PKG_BUILD_DIR)/data/IP.,XboxCn1.txt XboxCn2.txt XboxApp.txt Akamai.txt AkamaiV2.txt) $(1)/usr/share/xboxdownload/ip/
endef

$(eval $(call BuildPackage,xboxdownload))

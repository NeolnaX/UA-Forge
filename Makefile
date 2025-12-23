include $(TOPDIR)/rules.mk

PKG_NAME:=uaforge
PKG_VERSION:=0.1.1
PKG_RELEASE:=1

PKG_MAINTAINER:=NeolnaX <NeolnaX@outlook.com>
PKG_LICENSE:=GPL-3.0-only
PKG_LICENSE_FILES:=LICENSE

PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1
PKG_BUILD_FLAGS:=no-mips16

include $(INCLUDE_DIR)/package.mk
include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk

MAKE_PATH:=.
RUST_PKG_LOCKED:=1

# 注入版本号到 Rust 编译
CARGO_VARS += \
	CARGO_PKG_VERSION=$(PKG_VERSION)

define Package/uaforge
	SECTION:=net
	CATEGORY:=Network
	SUBMENU:=Web Servers/Proxies
	TITLE:=UAForge - User-Agent Proxy
	URL:=https://github.com/NeolnaX/UA-Forge
	DEPENDS:=$(RUST_ARCH_DEPENDS) +luci-compat
	CONFLICTS:=UAmask UAmask-rs UAmask-ipt ua3f-tproxy ua3f-tproxy-ipt
endef

define Package/uaforge/description
	A transparent proxy for modifying HTTP User-Agent (Rust implementation).
	Includes LuCI UI and init script for OpenWrt, and supports nftables/iptables set bypass.
endef

define Build/Prepare
	$(INSTALL_DIR) $(PKG_BUILD_DIR)
	$(CP) $(CURDIR)/src $(PKG_BUILD_DIR)/
	$(CP) $(CURDIR)/Cargo.toml $(PKG_BUILD_DIR)/
	$(CP) $(CURDIR)/Cargo.lock $(PKG_BUILD_DIR)/
	$(CP) $(CURDIR)/LICENSE $(PKG_BUILD_DIR)/
	# vendor and .cargo will be copied by workflow if they exist
	[ -d $(CURDIR)/vendor ] && $(CP) $(CURDIR)/vendor $(PKG_BUILD_DIR)/ || true
	[ -d $(CURDIR)/.cargo ] && $(CP) $(CURDIR)/.cargo $(PKG_BUILD_DIR)/ || true
endef

# Prefer vendored dependencies; cargo should not need network.
CARGO_PKG_ARGS += --offline

define Package/uaforge/conffiles
/etc/config/uaforge
endef

define Package/uaforge/install
	$(INSTALL_DIR) $(1)/usr/bin/
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/uaforge $(1)/usr/bin/uaforge

	$(INSTALL_DIR) $(1)/etc/init.d/
	$(INSTALL_BIN) ./files/uaforge.init $(1)/etc/init.d/uaforge

	$(INSTALL_DIR) $(1)/etc/config/
	$(INSTALL_CONF) ./files/uaforge.uci $(1)/etc/config/uaforge

	$(INSTALL_DIR) $(1)/usr/lib/lua/luci/model/cbi/
	$(INSTALL_CONF) ./files/luci/cbi.lua $(1)/usr/lib/lua/luci/model/cbi/uaforge.lua

	$(INSTALL_DIR) $(1)/usr/lib/lua/luci/controller/
	$(INSTALL_CONF) ./files/luci/controller.lua $(1)/usr/lib/lua/luci/controller/uaforge.lua
endef

$(eval $(call BuildPackage,uaforge))

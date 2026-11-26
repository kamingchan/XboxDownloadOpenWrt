include $(TOPDIR)/rules.mk

LUCI_TITLE:=Xbox download acceleration
LUCI_DEPENDS:=+luci-base +xboxdownload
LUCI_PKGARCH:=all
PKG_LICENSE:=GPL-3.0-only
PKG_RELEASE:=1

include $(if $(wildcard $(TOPDIR)/feeds/luci/luci.mk),$(TOPDIR)/feeds/luci/luci.mk,../../luci.mk)

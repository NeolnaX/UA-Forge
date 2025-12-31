module("luci.controller.uaforge", package.seeall)

function index()
	local e = entry({ "admin", "services", "uaforge" }, cbi("uaforge"), "UAForge", 90)
	e.dependent = true
	e.acl_depends = { "luci-app-uaforge" }
end

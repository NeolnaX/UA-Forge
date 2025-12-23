module("luci.controller.uaforge", package.seeall)

function index()
    entry({"admin", "services", "uaforge"}, cbi("uaforge"), "UAForge", 1)
end
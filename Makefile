.PHONY: mac-release windows-release

mac-release:
	npm run build:macos

windows-release:
	npm run build:windows

"""Regenerate the Finder layout with ds-store==1.3.1 (development only).

A solid background avoids machine-specific aliases and works without Finder,
AppleScript permissions, or pip dependencies on the release runner.
"""
from pathlib import Path

from ds_store import DSStore


def main():
    destination = Path(__file__).with_name("finder-layout.dsstore")
    with DSStore.open(str(destination), "w+") as store:
        store["."]["vSrn"] = ("long", 1)
        store["."]["icvl"] = ("type", b"icnv")
        store["."]["bwsp"] = {
            "WindowBounds": "{{240, 160}, {640, 400}}",
            "ShowStatusBar": False,
            "ShowTabView": False,
            "ShowToolbar": False,
            "ShowPathbar": False,
            "ShowSidebar": False,
            "ContainerShowSidebar": False,
            "PreviewPaneVisibility": False,
            "SidebarWidth": 0,
        }
        store["."]["icvp"] = {
            "viewOptionsVersion": 1,
            "backgroundType": 1,
            "backgroundColorRed": 245 / 255,
            "backgroundColorGreen": 244 / 255,
            "backgroundColorBlue": 240 / 255,
            "gridOffsetX": 0.0,
            "gridOffsetY": 0.0,
            "gridSpacing": 100.0,
            "arrangeBy": "none",
            "showIconPreview": False,
            "showItemInfo": False,
            "labelOnBottom": True,
            "textSize": 13.0,
            "iconSize": 96.0,
            "scrollPositionX": 0.0,
            "scrollPositionY": 0.0,
        }
        store["墨砚.app"]["Iloc"] = (180, 120)
        store["Applications"]["Iloc"] = (460, 120)
        store["使用指南与许可"]["Iloc"] = (320, 290)


if __name__ == "__main__":
    main()

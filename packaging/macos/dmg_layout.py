"""Write Finder metadata against the mounted volume so background aliases resolve."""
from ds_store import DSStore
from mac_alias import Alias


def write_layout(root):
    destination = root / ".DS_Store"
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
            "backgroundType": 2,
            "backgroundImageAlias": Alias.for_file(str(root / ".background.tiff")).to_bytes(),
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
        store["墨砚.app"]["Iloc"] = (180, 172)
        store["Applications"]["Iloc"] = (460, 172)

"""Standard Finder installation layout; dmgbuild writes it without a GUI."""

files = [defines["app"]]  # noqa: F821 - dmgbuild injects defines when executing this file.
symlinks = {"Applications": "/Applications"}
format = "UDZO"
background = "builtin-arrow"
window_rect = ((100, 100), (640, 280))
default_view = "icon-view"
icon_locations = {"Caper.app": (140, 120), "Applications": (500, 120)}
icon_size = 96
text_size = 14
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False

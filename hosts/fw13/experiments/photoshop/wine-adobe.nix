{ wineWow64Packages }:

# nixpkgs' Wine unstable (which already carries the upstreamed mshtml/jscript half
# of the Adobe Creative Cloud installer fix, merged into wine master Feb 2026) plus
# the three patches from PhialsBasement's wine-11.10-adobe fork still out of tree:
#   0001 libs/xml2 — tolerate Adobe's embedded <?xml?> declarations (winehq !10025, still open)
#   0002 msvcrt    — guard a NULL deref that crashes the installer
#   0003 d2d1      — draw geometry realizations to bitmap targets (canvas rendering)
#
# gstreamerSupport makes nixpkgs wrap bin/wine in a bash script (to set the plugin
# path); winetricks probes Wine's arch with `file` and chokes on a non-ELF wine, so
# disable it. The only loss is media codecs, which Photoshop's image editing doesn't use.
(wineWow64Packages.unstableFull.override { gstreamerSupport = false; }).overrideAttrs (old: {
  pname = "wine-adobe";
  patches = (old.patches or [ ]) ++ [
    ./patches/0001-libs-xml2-tolerate-embedded-xml-declarations.patch
    ./patches/0002-msvcrt-avoid-null-deref-in-_FindAndUnlinkFrame.patch
    ./patches/0003-d2d1-render-geometry-realizations-to-bitmap-targets.patch
  ];
})

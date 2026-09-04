final: prev: {
  # sip 6.16 miscompiles legacy (ABI v12) bindings, breaking pyqt5 on python 3.14.
  # Merged as nixpkgs 7499398, drop once nixos-unstable catches up.
  pythonPackagesExtensions = prev.pythonPackagesExtensions ++ [
    (pyfinal: pyprev: {
      sip = pyprev.sip.overrideAttrs (old: {
        patches = old.patches or [ ] ++ [
          (final.fetchpatch {
            name = "legacy-api-binding-fix.patch";
            url = "https://github.com/Python-SIP/sip/commit/09598895c607f3e41f0249ade217ace0a4da6437.patch";
            hash = "sha256-v0YeHyg0ymB0v32gpVRbMBIUk9U2etjs93VuOGPGg2M=";
          })
        ];
      });
    })
  ];
}

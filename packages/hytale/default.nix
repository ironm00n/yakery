{
  stdenv,
  fetchurl,

  ostree,
  flatpak,
  buildFHSEnv,

  pkgs,
}:
let
  unpackFlatpack =
    { name, src }:
    stdenv.mkDerivation {
      inherit name src;

      nativeBuildInputs = [
        ostree
        flatpak
      ];

      unpackPhase = ''
        runHook preUnpack

        mkdir repo

        echo "Initializing repository..."
        ostree init --repo=repo --mode=archive

        echo "Importing image..."
        flatpak build-import-bundle repo $src

        launcher_ref="$(ostree refs --repo=repo | grep com.hypixel.HytaleLauncher | head -n1)"

        echo "Checking out image $launcher_ref..."
        ostree checkout --repo=repo --user-mode "$launcher_ref" $out

        runHook postUnpack
      '';

      dontBuild = true;
      dontConfigure = true;
      dontInstall = true;
    };

  hytale-launcher-unwrapped = stdenv.mkDerivation rec {
    name = "hytale-launcher";
    src = unpackFlatpack {
      inherit name;
      src = fetchurl {
        url = "https://launcher.hytale.com/builds/release/linux/amd64/hytale-launcher-latest.flatpak";
        hash = "sha256-GRkA9piBoPoCLoxM5zZTIv7ocdIClvsavGnjah0JJME=";
      };
    };

    installPhase = ''
      cp -r $src/files $out

      chmod -R +w $out
      cp -r $src/export/share $out

      sed -i "s#/app/bin/hytale-launcher#$out/bin/hytale-launcher#g" $out/bin/hytale-launcher-wrapper
    '';

    dontBuild = true;
    dontConfigure = true;
  };

  hytale-launcher-fhs-env = buildFHSEnv {
    name = "hytale-launcher-wrapper";

    runScript = pkgs.writeScript "hytale-launcher-wrapper-preconfigure" ''
      export LD_LIBRARY_PATH=/run/opengl-driver/lib:/run/opengl-driver-32/lib:$LD_LIBRARY_PATH
      export LIBGL_DRIVERS_PATH=/run/opengl-driver/lib/dri:/run/opengl-driver-32/lib/dri
      export __EGL_VENDOR_LIBRARY_DIRS=/run/opengl-driver/share/glvnd/egl_vendor.d:/run/opengl-driver-32/share/glvnd/egl_vendor.d

      # Otherwise WebkitGTK just crashes with:
      # Failed to create GBM buffer of size 1026x640: Invalid argument. <- X11
      # Gdk-Message: 17:26:21.876: Error 71 (Protocol error) dispatching to Wayland display. <- Wayland
      export WEBKIT_DISABLE_COMPOSITING_MODE=1

      if [[ "$HYTALE_FHS_BASH" -eq 1 ]]; then
        exec bash "$@"
      fi

      exec "${hytale-launcher-unwrapped}/bin/hytale-launcher-wrapper" "$@"
    '';

    # https://github.com/NixOS/nixpkgs/issues/372135
    extraBwrapArgs = [
      "--ro-bind-try /etc/egl/egl_external_platform.d /etc/egl/egl_external_platform.d"
    ];

    targetPkgs =
      pkgs: with pkgs; [
        # Launcher
        glib
        gtk3
        webkitgtk_4_1
        gdk-pixbuf
        libsoup_3

        # Game
        alsa-lib
        vulkan-loader

        libGL

        udev
        libudev0-shim

        wayland
        wayland-protocols
        libxkbcommon
        libdecor

        icu
        pipewire
        openssl_3

        xorg.libX11
        xorg.libXext
      ];
  };
in
stdenv.mkDerivation {
  name = "hytale-launcher";
  phases = [ "installPhase" ];

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    ln -s ${hytale-launcher-fhs-env}/bin/hytale-launcher-wrapper $out/bin/hytale-launcher-wrapper
    ln -s ${hytale-launcher-unwrapped}/share $out/share

    runHook postInstall
  '';
}

{ pkgs }:
let
  app =
    name:
    pkgs.makeDesktopItem {
      inherit name;
      desktopName = name;
      exec = "true";
    };

  # KService::serviceByDesktopName goes through KSycoca::ensureCacheValid, the
  # same path Dolphin's "Open With" and Kicker take.
  probe = pkgs.stdenv.mkDerivation {
    name = "ksycoca-probe";
    src = pkgs.linkFarm "ksycoca-probe-src" [
      {
        name = "CMakeLists.txt";
        path = pkgs.writeText "CMakeLists.txt" ''
          cmake_minimum_required(VERSION 3.16)
          project(ksycoca-probe CXX)
          find_package(ECM REQUIRED NO_MODULE)
          set(CMAKE_MODULE_PATH ''${ECM_MODULE_PATH})
          find_package(Qt6 REQUIRED COMPONENTS Core)
          find_package(KF6Service REQUIRED)
          add_executable(ksycoca-probe probe.cpp)
          target_link_libraries(ksycoca-probe KF6::Service Qt6::Core)
          install(TARGETS ksycoca-probe DESTINATION bin)
        '';
      }
      {
        name = "probe.cpp";
        path = pkgs.writeText "probe.cpp" ''
          #include <KService>
          #include <QCoreApplication>
          int main(int argc, char **argv)
          {
              QCoreApplication app(argc, argv);
              return KService::serviceByDesktopName(QString::fromLocal8Bit(argv[1])) ? 0 : 1;
          }
        '';
      }
    ];
    nativeBuildInputs = [
      pkgs.cmake
      pkgs.kdePackages.extra-cmake-modules
    ];
    buildInputs = [
      pkgs.kdePackages.qtbase
      pkgs.kdePackages.kservice
      pkgs.kdePackages.kconfig
      pkgs.kdePackages.kcoreaddons
    ];
    dontWrapQtApps = true;
  };

  # kbuildsycoca registers applications only through a menu file's
  # <DefaultAppDirs/>. The nodes take the bundle's file but keep it
  # store-backed: under /etc it would live in a real directory that setup-etc
  # rewrites on every activation, and that mtime bump alone makes unpatched
  # kservice rebuild, hiding the bug the unpatched node is there to show.
  common =
    { config, ... }:
    {
      imports = [ ../../bundles/xdg-menu.nix ];
      virtualisation.graphics = false;
      users.users.alice.isNormalUser = true;
      environment.systemPackages = [
        probe
        (app "org.example.base")
        (pkgs.runCommand "xdg-menu" { } ''
          install -Dm444 ${config.bundles.xdg-menu.source} $out/etc/xdg/menus/applications.menu
        '')
      ];
      specialisation.gen2.configuration.environment.systemPackages = [ (app "org.example.added") ];
    };

  asAlice =
    cmd:
    "su alice -c 'XDG_DATA_DIRS=/run/current-system/sw/share XDG_CONFIG_DIRS=/run/current-system/sw/etc/xdg ${cmd}'";
  probeFor = name: asAlice "ksycoca-probe ${name}";
  cacheInode = "stat -c %i /home/alice/.cache/ksycoca6_*";
in
pkgs.testers.runNixOSTest {
  name = "ksycoca-bundle";

  nodes = {
    patched = {
      imports = [
        common
        ../../bundles/ksycoca
      ];
      bundles.ksycoca.enable = true;
    };
    unpatched = common;
  };

  testScript = ''
    for m in [patched, unpatched]:
        m.wait_for_unit("multi-user.target")
        m.succeed("${probeFor "org.example.base"}")
        m.succeed("${cacheInode}")
        m.fail("${probeFor "org.example.added"}")
        m.succeed("/run/current-system/specialisation/gen2/bin/switch-to-configuration test")
        m.succeed("test -e /run/current-system/sw/share/applications/org.example.added.desktop")

    with subtest("a pre-existing cache notices the new generation"):
        patched.succeed("${probeFor "org.example.added"}")

    with subtest("and is not rebuilt again on the next lookup"):
        inode = patched.succeed("${cacheInode}")
        patched.succeed("${probeFor "org.example.added"}")
        assert inode == patched.succeed("${cacheInode}"), "cache was rebuilt a second time"

    with subtest("nixpkgs' kservice still has the bug, so the bundle is still needed"):
        unpatched.fail("${probeFor "org.example.added"}")
        unpatched.succeed("rm /home/alice/.cache/ksycoca6_*")
        unpatched.succeed("${probeFor "org.example.added"}")
  '';
}

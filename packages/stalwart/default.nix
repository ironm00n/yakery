{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  protobuf,
  bzip2,
  openssl,
  sqlite,
  foundationdb,
  zstd,
  stdenv,
  nix-update-script,
  nixosTests,
  rocksdb,
  callPackage,
  runCommand,
  gzip,
  jq,
  withFoundationdb ? false,
  stalwartEnterprise ? false,
  buildPackages,
  webuiOauthClientId
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "stalwart" + (lib.optionalString stalwartEnterprise "-enterprise");
  version = "0.16.24";

  src = fetchFromGitHub {
    owner = "stalwartlabs";
    repo = "stalwart";
    rev = "v${finalAttrs.version}";
    hash = "sha256-/QAwTzP9Z/+WeU91ZMXFrBcFrH6IGWd64/LdPvKf/BA=";
  };

  cargoHash = "sha256-5sZI34XRs6AICCCAJ61plhUzQR5Nhgtw+uIIsEkDRvI=";

  depsBuildBuild = [
    pkg-config
    zstd
  ];

  nativeBuildInputs = [
    protobuf
    rustPlatform.bindgenHook
  ];

  buildInputs = [
    bzip2
    openssl
    sqlite
    zstd
  ]
  ++ lib.optionals (stdenv.hostPlatform.isLinux && withFoundationdb) [ foundationdb ];

  nativeCheckInputs = [
    openssl
  ];

  # Issue: https://github.com/stalwartlabs/stalwart/issues/1104
  buildNoDefaultFeatures = true;
  buildFeatures = [
    # "sqlite"
    # "postgres"
    # "mysql"
    "rocks"
    "s3"
    # "redis"
    # "azure"
    # "nats"
  ]
  ++ lib.optionals withFoundationdb [ "foundationdb" ]
  ++ lib.optionals stalwartEnterprise [ "enterprise" ];

  env = {
    OPENSSL_NO_VENDOR = true;
    ZSTD_SYS_USE_PKG_CONFIG = true;
    ROCKSDB_INCLUDE_DIR = "${rocksdb}/include";
    ROCKSDB_LIB_DIR = "${rocksdb}/lib";
    CARGO_PROFILE_RELEASE_LTO = "false";
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
  }
  //
    lib.optionalAttrs
      (stdenv.hostPlatform.isLinux && (stdenv.hostPlatform.isAarch64 || stdenv.hostPlatform.isArmv7))
      {
        JEMALLOC_SYS_WITH_LG_PAGE = 16;
      };

  postInstall = ''
    mkdir -p $out/lib/systemd/system

    substitute resources/systemd/stalwart-mail.service $out/lib/systemd/system/stalwart.service \
      --replace-fail "__PATH__/bin/stalwart" "$out/bin/stalwart" \
      --replace-fail "__PATH__/etc/config.json" "/etc/stalwart/config.json";
  '';

  doCheck = false;

  passthru = {
    inherit rocksdb; # make used rocksdb version available (e.g., for backup scripts)
    webui = buildPackages.callPackage ./webui.nix { oauthClientId = webuiOauthClientId; };
    spam-filter = callPackage ./spam-filter.nix { };
    cli = callPackage ./cli.nix {};

    # Stalwart's full permission registry as { wireName = "wireName"; ... },
    # extracted from the JSON config schema it ships in its own source tree
    # (resources/schema/schema.json.gz -- the file served at GET /api/schema).
    # Lets NixOS role definitions name permissions via `with`, so a typo is an
    # eval-time error. Derived from `src`: always exact for this version.
    permissions =
      let
        names = runCommand "stalwart-permission-names.json" { } ''
          ${gzip}/bin/gunzip -c ${finalAttrs.src}/resources/schema/schema.json.gz \
            | ${jq}/bin/jq '[ .enums.Permission[].name ] | sort' > $out
        '';
      in
      lib.genAttrs (lib.importJSON names) lib.id;

    updateScript = nix-update-script { };
    tests.stalwart = nixosTests.stalwart;
  };

  meta = {
    description = "Secure, modern, all-in-one mail and collaboration server";
    longDescription = ''
      Secure, scalable and fluent in every protocol (IMAP, JMAP, SMTP, CalDAV, CardDAV, WebDAV).
    '';
    homepage = "https://github.com/stalwartlabs/stalwart";
    changelog = "https://github.com/stalwartlabs/stalwart/blob/main/CHANGELOG.md";
    license = [
      lib.licenses.agpl3Only
    ]
    ++ lib.optionals stalwartEnterprise [
      {
        fullName = "Stalwart Enterprise License 1.0 (SELv1) Agreement";
        url = "https://github.com/stalwartlabs/stalwart/blob/main/LICENSES/LicenseRef-SEL.txt";
        free = false;
        redistributable = false;
      }
    ];

    mainProgram = "stalwart";
    maintainers = with lib.maintainers; [
      happysalada
      onny
      oddlama
      pandapip1
      norpol
    ];
  };
})


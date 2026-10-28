{
  lib,
  applyPatches,
  buildGoModule,
  callPackage,
  fetchFromGitHub,
  fetchpatch,

  buf,
  dart-sass,
  grpc-gateway,
  protoc-gen-connect-go,
  protoc-gen-go,
  protoc-gen-go-grpc,
  protoc-gen-validate,
  statik,
  writableTmpDirAsHomeHook,
}:

let
  version = "4.19.4";
  commit = "101343ad87aef7b0eea7c3522d519308e0ea9547";
  commitDate = "2026-10-01T08:44:27Z";

  carriedCommits = [
    {
      # zitadel/zitadel#12180: scope claim in JWT access tokens
      rev = "d1adf4d003f591eb5c098ac1eef7407f509c5942";
      files = [ "internal/api/oidc/token.go" ];
      hash = "sha256-BJsJXvgmM8RMpq4xDdBQ/Y+Ee/iAkPpgjr6QqsyJLGE=";
    }
  ];

  shortRev = builtins.substring 0 8;
  buildVersion =
    "v${version}"
    + lib.optionalString (carriedCommits != [ ]) (
      "+" + lib.concatMapStringsSep "." (c: shortRev c.rev) carriedCommits
    );

  src = applyPatches {
    name = "zitadel-${version}-source";
    src = fetchFromGitHub {
      owner = "zitadel";
      repo = "zitadel";
      rev = commit;
      hash = "sha256-gfeJXRBVZNkGl+SHsGtOMLaNiksg/sFnzf8ISb5cItM=";
    };
    patches = map (
      c:
      fetchpatch {
        url = "https://github.com/zitadel/zitadel/commit/${c.rev}.patch";
        includes = c.files;
        inherit (c) hash;
      }
    ) carriedCommits;
  };

  goModules = {
    inherit version src;
    proxyVendor = true;
    vendorHash = "sha256-8/TkV1JTKNSIknzEPbTDWzcOZQ6jCEkc6vSLLhdLYrs=";
  };

  protoImage = callPackage ./proto-image.nix { inherit version src; };

  protocPlugins = buildGoModule (
    goModules
    // {
      pname = "zitadel-protoc-plugins";
      subPackages = [
        "internal/protoc/protoc-gen-authoption"
        "internal/protoc/protoc-gen-zitadel"
      ];
    }
  );

  console = callPackage ./console.nix {
    inherit version src protoImage;
    protoc-gen-js = callPackage ../protoc-gen-js { };
  };
in
buildGoModule (
  goModules
  // {
    pname = "zitadel";

    subPackages = [ "." ];
    env.CGO_ENABLED = 0;
    ldflags = [
      "-s"
      "-w"
      "-X github.com/zitadel/zitadel/cmd/build.commit=${shortRev commit}"
      "-X github.com/zitadel/zitadel/cmd/build.date=${commitDate}"
      "-X github.com/zitadel/zitadel/cmd/build.version=${buildVersion}"
    ];

    nativeBuildInputs = [
      buf
      dart-sass
      grpc-gateway
      protoc-gen-connect-go
      protoc-gen-go
      protoc-gen-go-grpc
      protoc-gen-validate
      protocPlugins
      statik
      writableTmpDirAsHomeHook
    ];

    # apps/api/project.json: generate-stubs, generate-statik, generate-assets, build-console
    preBuild = ''
      buf generate ${protoImage}/zitadel.binpb
      mkdir -p pkg/grpc openapi/v2/zitadel
      cp -r .artifacts/grpc/github.com/zitadel/zitadel/pkg/grpc/* pkg/grpc/
      cp -r .artifacts/grpc/zitadel/ openapi/v2/zitadel

      (cd internal/api/ui/login/static/resources && sass themes/scss/zitadel.scss themes/zitadel/css/zitadel.css)
      go generate internal/api/ui/login/statik/generate.go
      go generate internal/notification/statik/generate.go
      go generate internal/statik/generate.go

      mkdir -p apps/docs/content/apis/assets
      go run internal/api/assets/generator/asset_generator.go \
        -directory=internal/api/assets/generator/ \
        -assets=apps/docs/content/apis/assets/assets.mdx

      find internal/api/ui/console/static -mindepth 1 ! -name gitkeep -delete
      cp -r ${console}/* internal/api/ui/console/static
    '';
    # buildGoModule copies preBuild into the fixed-output module fetch
    overrideModAttrs = {
      preBuild = "";
    };

    doCheck = false;

    passthru = {
      inherit
        buildVersion
        console
        protoImage
        protocPlugins
        ;
    };

    meta = {
      description = "Identity and access management platform";
      homepage = "https://zitadel.com/";
      downloadPage = "https://github.com/zitadel/zitadel/releases";
      platforms = lib.platforms.linux;
      license = lib.licenses.agpl3Only;
      sourceProvenance = [ lib.sourceTypes.fromSource ];
      mainProgram = "zitadel";
    };
  }
)

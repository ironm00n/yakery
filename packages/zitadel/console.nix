{
  stdenv,
  fetchPnpmDeps,
  pnpmConfigHook,
  pnpm_10,
  nodejs_22,

  buf,
  grpc-gateway,
  protoc-gen-grpc-web,

  version,
  src,
  protoImage,
  protoc-gen-js,
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "zitadel-console";
  inherit version src;

  pnpmWorkspaces = [ "@zitadel/console..." ];
  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs)
      pname
      version
      src
      pnpmWorkspaces
      ;
    pnpm = pnpm_10;
    fetcherVersion = 4;
    hash = "sha256-KxQwsQb+ycsx5b0rg15HBpy/nS0xAk1I/GIOvc4dsF0=";
  };

  nativeBuildInputs = [
    nodejs_22
    pnpm_10
    pnpmConfigHook

    buf
    grpc-gateway
    protoc-gen-grpc-web
    protoc-gen-js
  ];

  # nx's build order for @zitadel/console: @zitadel/proto:generate,
  # @zitadel/client:build, @zitadel/console:generate, @zitadel/console:build
  buildPhase = ''
    runHook preBuild
    (cd packages/zitadel-proto && PATH=$PWD/node_modules/.bin:$PATH buf generate ${protoImage}/zitadel.binpb)
    pnpm --filter=@zitadel/client run build
    (cd console && buf generate ${protoImage}/zitadel.binpb --include-imports --include-wkt)
    pnpm --filter=@zitadel/console run build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    cp -r console/dist/console $out
    runHook postInstall
  '';
})

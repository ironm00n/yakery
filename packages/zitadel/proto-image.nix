{
  lib,
  stdenvNoCC,
  buf,
  cacert,
  yq-go,
  writableTmpDirAsHomeHook,

  version,
  src,
}:

let
  # An image holds buf's own copy of the well-known types, so its bytes change
  # with buf; only the registry's files are fixed-output.
  imports = stdenvNoCC.mkDerivation {
    pname = "zitadel-proto-imports";
    inherit version src;

    nativeBuildInputs = [
      buf
      cacert
      writableTmpDirAsHomeHook
    ];
    impureEnvVars = lib.fetchers.proxyImpureEnvVars;

    buildPhase = ''
      runHook preBuild
      buf export proto --output $out
      rm -r $out/zitadel
      runHook postBuild
    '';

    dontInstall = true;
    dontFixup = true;

    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-CzKVVouVreyLKdi1qe5XPBWT1NChhZQm+BazSpNOsZk=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "zitadel-proto-image";
  inherit version src;

  nativeBuildInputs = [
    buf
    yq-go
    writableTmpDirAsHomeHook
  ];

  postPatch = ''
    ln -s ${imports} imports
    yq -i '.directories += ["imports"]' buf.work.yaml
    yq -i 'del(.deps)' proto/buf.yaml
    rm proto/buf.lock
  '';

  buildPhase = ''
    runHook preBuild
    buf build proto --output zitadel.binpb
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm444 zitadel.binpb -t $out
    runHook postInstall
  '';

  passthru = { inherit imports; };
}

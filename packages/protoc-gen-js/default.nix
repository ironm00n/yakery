{
  lib,
  stdenv,
  fetchFromGitHub,
  pkg-config,
  protobuf_33,
  versionCheckHook,
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "protoc-gen-js";
  version = "4.0.2";

  src = fetchFromGitHub {
    owner = "protocolbuffers";
    repo = "protobuf-javascript";
    tag = "v${finalAttrs.version}";
    hash = "sha256-vFTR7dwquZht4st90bFQ9CJMYBPbL+DWO0uQ3xsXIBU=";
  };

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ protobuf_33 ];

  # the two cc_binary targets of generator/BUILD.bazel, without Bazel
  buildPhase = ''
    runHook preBuild
    $CXX -std=c++17 -O2 generator/generate-version-header.cc -o generate-version-header
    ./generate-version-header package.json generator/version.h
    $CXX -std=c++17 -O2 -I. \
      generator/js_generator.cc generator/protoc-gen-js.cc generator/well_known_types_embed.cc \
      -o protoc-gen-js $(pkg-config --cflags --libs protobuf) -lprotoc
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 protoc-gen-js -t $out/bin
    runHook postInstall
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = true;

  meta = {
    description = "Protobuf plugin for generating JavaScript code";
    mainProgram = "protoc-gen-js";
    homepage = "https://github.com/protocolbuffers/protobuf-javascript";
    platforms = lib.platforms.linux;
    license = with lib.licenses; [
      asl20
      bsd3
    ];
    sourceProvenance = [ lib.sourceTypes.fromSource ];
  };
})

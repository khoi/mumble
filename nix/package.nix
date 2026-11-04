{
  lib,
  rustPlatform,
  pkg-config,
  python3,
  vulkan-loader,
  alsa-lib,
  alsa-plugins,
  pipewire,
  openssl,
  libxkbcommon,
  wayland,
  fontconfig,
  freetype,
  curl,
  util-linux,
  wl-clipboard,
  wtype,
  makeWrapper,
  autoAddDriverRunpath,
  symlinkJoin,
  libx11,
  libxcb,
  libxcursor,
  libxrandr,
  libxi,
  mkShell,
  rust-analyzer,
  rustfmt,
  clippy,
}:

let
  buildEnv = {
    OPENSSL_NO_VENDOR = "1";
  };
  runtimeEnv = {
    LD_LIBRARY_PATH = lib.makeLibraryPath [
      vulkan-loader
      alsa-lib
      wayland
      libxkbcommon
      fontconfig
    ];
    ALSA_PLUGIN_DIR = symlinkJoin {
      name = "mumble-alsa-plugins";
      paths = [
        "${pipewire}/lib/alsa-lib"
        "${alsa-plugins}/lib/alsa-lib"
      ];
    };
  };
  runtimePackages = [
    curl
    wl-clipboard
    wtype
  ];
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "mumble";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../.cargo
      ../build.rs
      ../src
      ../tests
      ../native
      ../resources
      ../assets
      ../packaging
      ../macos/MumbleMicrophoneDriver/Info.plist
      # Built in: a 2.3.1 install moves itself into this installer's layout.
      ../.github/release/install-linux.sh
    ];
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    outputHashes = {
      "rune-0.0.1" = "sha256-V4z8nWvZjj3p4dZZOaFTengYIYqwOQu8aktCJvtNxwA=";
    };
  };

  nativeBuildInputs = [
    pkg-config
    python3
    makeWrapper
    autoAddDriverRunpath
  ];
  buildInputs = [
    vulkan-loader
    alsa-lib
    openssl
    libxkbcommon
    wayland
    fontconfig
    freetype
    libx11
    libxcb
    libxcursor
    libxrandr
    libxi
  ];

  # OpenCode HTTP requests run through curl, including in the tests; the
  # 2.3.1 adoption test runs the Linux installer, which needs flock.
  nativeCheckInputs = [
    curl
    util-linux
  ];

  env = buildEnv;
  # Desktop and installed-model tests are #[ignore]; run the remaining suite.
  doCheck = true;
  preCheck = ''
    export HOME="$TMPDIR/mumble-test-home"
    mkdir -p "$HOME"
  '';

  postInstall = ''
    install -Dm644 ${../packaging/anomaly.mumble.desktop} $out/share/applications/anomaly.mumble.desktop
    mkdir -p $out/share/icons
    cp -R ${../packaging/linux/icons/hicolor} $out/share/icons/hicolor
    substituteInPlace $out/share/applications/anomaly.mumble.desktop \
      --replace-fail '@MUMBLE_BIN@' "$out/bin/mumble"
  '';

  postFixup = ''
    wrapProgram $out/bin/mumble \
      --prefix PATH : ${lib.makeBinPath runtimePackages} \
      --prefix LD_LIBRARY_PATH : ${runtimeEnv.LD_LIBRARY_PATH} \
      --set ALSA_PLUGIN_DIR ${runtimeEnv.ALSA_PLUGIN_DIR}
  '';

  passthru.devShell = mkShell {
    inputsFrom = [ finalAttrs.finalPackage ];
    env = buildEnv // runtimeEnv;
    packages = runtimePackages ++ [
      rust-analyzer
      rustfmt
      clippy
    ];
  };

  meta = {
    description = "Local-first voice dictation (Linux beta)";
    homepage = "https://anoma.ly/mumble/";
    license = lib.licenses.mit;
    mainProgram = "mumble";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
})

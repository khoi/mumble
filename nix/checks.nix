{ self, pkgs }:
let
  inherit (pkgs) lib;
  mumble = self.packages.${pkgs.stdenv.hostPlatform.system}.mumble;
  shell = self.devShells.${pkgs.stdenv.hostPlatform.system}.default;
  home =
    settings:
    (self.inputs.home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        self.homeManagerModules.mumble
        {
          home.username = "mumble-test";
          home.homeDirectory = "/home/mumble-test";
          home.stateVersion = "26.05";
          programs.mumble = settings;
        }
      ];
    }).config;
  enabled = home {
    enable = true;
    autostart = true;
  };
  manual = home { enable = true; };
  disabled = home { enable = false; };
  declared = home {
    enable = true;
    shortcut = "control+option+space";
    provider = "codex";
    settings = {
      sound_effect_volume = 0.4;
      microphone = null;
      "transcription.language" = "de";
    };
  };
  declaredSettings = (pkgs.formats.json { }).generate "mumble-settings.json" {
    dictation_hotkey = "control+option+space";
    "transcription.provider" = "codex";
    "transcription.language" = "de";
    sound_effect_volume = 0.4;
    microphone = null;
  };
  custom = home {
    enable = true;
    autostart = true;
    package = pkgs.hello;
    systemdTarget = "sway-session.target";
  };
  system =
    (self.inputs.nixpkgs.lib.nixosSystem {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        self.nixosModules.mumble
        {
          programs.mumble.enable = true;
          system.stateVersion = "26.05";
        }
      ];
    }).config;
in
{
  package = pkgs.runCommand "mumble-package-check" { } ''
    export HOME="$TMPDIR/home"
    mkdir -p "$HOME"
    ${lib.getExe mumble} --version
    touch "$out"
  '';

  shell-environment =
    assert lib.all (name: shell.${name} == mumble.${name}) [
      "OPENSSL_NO_VENDOR"
    ];
    # OpenBLAS ordering belongs to build.rs, not an injected Nix-only workaround.
    assert !(mumble ? RUSTFLAGS) && !(shell ? RUSTFLAGS);
    assert lib.all
      (package: builtins.elem "${lib.getLib package}/lib" (lib.splitString ":" shell.LD_LIBRARY_PATH))
      [
        pkgs.vulkan-loader
        pkgs.wayland
        pkgs.libxkbcommon
      ];
    assert lib.all (package: builtins.elem (lib.getDev package) shell.nativeBuildInputs) [
      pkgs.curl
      pkgs.wl-clipboard
      pkgs.wtype
    ];
    pkgs.runCommand "mumble-shell-environment-check" { } ''
      test -d ${shell.ALSA_PLUGIN_DIR}
      touch "$out"
    '';

  modules =
    assert builtins.elem mumble system.environment.systemPackages;
    assert builtins.elem mumble enabled.home.packages;
    assert !(builtins.elem mumble disabled.home.packages);
    assert manual.systemd.user.services.mumble.Install.WantedBy == [ ];
    assert !(disabled.systemd.user.services ? mumble);
    assert enabled.systemd.user.services.mumble.Service.ExecStart == [ "${lib.getExe mumble} service" ];
    assert enabled.systemd.user.services.mumble.Service.Restart == "on-failure";
    assert enabled.systemd.user.services.mumble.Service.Type == "notify";
    assert enabled.systemd.user.services.mumble.Service.NotifyAccess == "main";
    assert builtins.elem "MUMBLE_APPLICATION_SUPPORT_DIR=${enabled.xdg.dataHome}/mumble"
      enabled.systemd.user.services.mumble.Service.Environment;
    assert custom.systemd.user.services.mumble.Service.ExecStart == [ "${lib.getExe pkgs.hello} service" ];
    assert lib.all (name: custom.systemd.user.services.mumble.Unit.${name} == [ "sway-session.target" ]) [
      "After"
      "PartOf"
    ];
    assert custom.systemd.user.services.mumble.Install.WantedBy == [ "sway-session.target" ];
    assert !(enabled.home.activation ? mumbleSettings);
    assert lib.hasInfix (builtins.unsafeDiscardStringContext "settings apply ${declaredSettings}") (
      builtins.unsafeDiscardStringContext declared.home.activation.mumbleSettings.data
    );
    assert lib.elem "reloadSystemd" declared.home.activation.mumbleSettings.after;
    pkgs.runCommand "mumble-module-check" { } ''
      bash -n ${enabled.systemd.user.services.mumble.Service.ExecStartPre}
      touch "$out"
    '';

  session-readiness =
    let
      xprop = pkgs.writeShellScriptBin "xprop" ''
        printf '%s\n' "''${MUMBLE_TEST_XPROP:-}"
      '';
      sleep = pkgs.writeShellScriptBin "sleep" "exit 0";
    in
    pkgs.runCommand "mumble-session-readiness-check" { } ''
      run() {
        env -i PATH=${
          lib.makeBinPath [
            xprop
            sleep
            pkgs.gnugrep
          ]
        } "$@" \
          ${pkgs.bash}/bin/bash -euo pipefail ${./wait-for-session.sh}
      }

      if run; then exit 1; fi
      if run DISPLAY=:1 MUMBLE_TEST_XPROP='_NET_SUPPORTING_WM_CHECK: not found.'; then exit 1; fi
      if run DISPLAY=:1 MUMBLE_TEST_XPROP='_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0x0'; then exit 1; fi
      run DISPLAY=:1 MUMBLE_TEST_XPROP='_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0xa00001'

      if run WAYLAND_DISPLAY=wayland-test XDG_RUNTIME_DIR="$TMPDIR"; then exit 1; fi
      ${pkgs.python3}/bin/python -c \
        'import socket, sys; sock = socket.socket(socket.AF_UNIX); sock.bind(sys.argv[1])' \
        "$TMPDIR/wayland-test"
      run WAYLAND_DISPLAY=wayland-test XDG_RUNTIME_DIR="$TMPDIR"
      run WAYLAND_DISPLAY="$TMPDIR/wayland-test"
      touch "$out"
    '';
}

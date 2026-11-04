{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.mumble;
  json = pkgs.formats.json { };
  supportDir = "${config.xdg.dataHome}/mumble";
  settings =
    lib.filterAttrs (_: value: value != null) {
      dictation_hotkey = cfg.shortcut;
      "transcription.provider" = cfg.provider;
      "transcription.language" = cfg.language;
    }
    // cfg.settings;
  waitForSession = pkgs.writeShellApplication {
    name = "mumble-wait-for-session";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.xprop
    ];
    text = builtins.readFile ./wait-for-session.sh;
  };
in
{
  options.programs.mumble = {
    enable = lib.mkEnableOption "Mumble voice dictation";
    package = lib.mkOption {
      type = lib.types.package;
      description = "Mumble package to install.";
    };
    autostart = lib.mkEnableOption "Mumble startup with the graphical session";
    systemdTarget = lib.mkOption {
      type = lib.types.str;
      default = "graphical-session.target";
      example = "sway-session.target";
      description = ''
        User target that owns the graphical session. The compositor or session
        manager must import its display environment before starting this target,
        and stop the target at logout. This module does not start a session.
      '';
    };
    settings = lib.mkOption {
      inherit (json) type;
      default = { };
      example = lib.literalExpression ''
        {
          sound_effect_volume = 0.4;
          shortcut_mode = "toggle";
          "labs.live_text" = true;
        }
      '';
      description = ''
        Settings applied on every activation, keyed as `mumble settings list`
        names them. Declared keys overwrite Mumble's values; settings changed in
        the window that aren't declared here keep their values. A running Mumble
        adopts them at once; otherwise they are written to settings.json.
      '';
    };
    shortcut = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "control+option+space";
      description = "Dictation shortcut, the `dictation_hotkey` setting.";
    };
    provider = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum [
        "claude"
        "codex"
      ]);
      default = null;
      example = "claude";
      description = ''
        The cloud service that transcribes dictations, the
        `transcription.provider` setting. Mumble uses the sign-in the chosen
        app (Claude Code or Codex) already holds.
      '';
    };
    language = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "de";
      description = "Dictation language, a code the selected provider supports.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.elem pkgs.stdenv.hostPlatform.system [
          "x86_64-linux"
          "aarch64-linux"
        ];
        message = "The Mumble Nix package supports x86_64-linux and aarch64-linux only.";
      }
    ];
    home.packages = [ cfg.package ];
    # After reloadSystemd, so a restarted Mumble is the one that answers.
    home.activation.mumbleSettings = lib.mkIf (settings != { }) (
      lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
        if ! run ${pkgs.coreutils}/bin/env MUMBLE_APPLICATION_SUPPORT_DIR=${lib.escapeShellArg supportDir} \
          ${lib.getExe cfg.package} settings apply ${json.generate "mumble-settings.json" settings}; then
          warnEcho "Mumble didn't apply programs.mumble.settings; see the message above."
        fi
      ''
    );
    systemd.user.services.mumble = {
      Unit = {
        Description = "Mumble voice dictation";
        After = [ cfg.systemdTarget ];
        PartOf = [ cfg.systemdTarget ];
      };
      Service = {
        Type = "notify";
        NotifyAccess = "main";
        ExecStartPre = lib.getExe waitForSession;
        ExecStart = "${lib.getExe cfg.package} service";
        Environment = [ "MUMBLE_APPLICATION_SUPPORT_DIR=${supportDir}" ];
        EnvironmentFile = "-${supportDir}/session.env";
        TimeoutStartSec = 35;
        Restart = "on-failure";
        RestartSec = 5;
        TimeoutStopSec = 30;
        UMask = "0077";
      };
      Install.WantedBy = lib.optionals cfg.autostart [ cfg.systemdTarget ];
    };
  };
}

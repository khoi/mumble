# Managing Mumble in an organization

Administrators can turn Mumble features off on managed Macs with a configuration
profile. Served at https://anoma.ly/mumble/docs/admin.

These keys are **macOS only**. Windows reads only `DisableUpdates` (a DWORD
under `HKLM` or `HKCU\SOFTWARE\Policies\Anomaly\Mumble`), and Linux reads none.

## Keys

All keys are Booleans in the `anomaly.mumble` preference domain. A key counts only
when a configuration profile forces it (`CFPreferencesAppValueIsForced`), so a
value someone writes with `defaults write` locks nothing. Mumble reads them at
launch; a changed profile applies after Mumble relaunches.

| Key | Effect |
| --- | --- |
| `DisableOpenCodeProcessing` | No OpenCode requests: modes never rewrite with OpenCode. |
| `DisableTransforms` | No transforms at all, built in or from `mumble.config.ts`; the config is never loaded. |
| `RestrictTransformsToLoopback` | Transforms stay on, but `fetch` reaches only this Mac and `node:child_process` (`exec`, `execFile`) is refused. Every host name, redirects included, must resolve to loopback addresses only (`127.0.0.0/8`, `::1`); the check runs on the resolved addresses Mumble connects to, and the system proxy is bypassed. Use it with a local, authenticated model proxy. |
| `DisableLabs` | Every Labs feature is off. |
| `DisableUpdates` | Mumble never checks for, downloads, or installs updates. |

A locked feature reads as off everywhere: the window shows its setting off
and disabled with "Managed by your organization", and `mumble settings set`,
`mumble labs on`, the local API, shortcuts, and background work all refuse it.
Mumble never rewrites the person's own settings, so removing the profile brings
back what they had chosen.

## Sample profile

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadDisplayName</key>
  <string>Mumble</string>
  <key>PayloadIdentifier</key>
  <string>com.example.mumble</string>
  <key>PayloadType</key>
  <string>Configuration</string>
  <key>PayloadUUID</key>
  <string>9A1C2E3F-0000-4000-8000-000000000001</string>
  <key>PayloadVersion</key>
  <integer>1</integer>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadType</key>
      <string>anomaly.mumble</string>
      <key>PayloadIdentifier</key>
      <string>com.example.mumble.settings</string>
      <key>PayloadUUID</key>
      <string>9A1C2E3F-0000-4000-8000-000000000002</string>
      <key>PayloadVersion</key>
      <integer>1</integer>
      <key>DisableOpenCodeProcessing</key>
      <true/>
      <key>RestrictTransformsToLoopback</key>
      <true/>
      <key>DisableLabs</key>
      <true/>
    </dict>
  </array>
</dict>
</plist>
```

Replace the identifiers and UUIDs with your own and leave out the keys you
don't need. Most MDM tools (Jamf, Kandji, Intune, Mosyle) accept the inner
dictionary as a custom settings payload for the `anomaly.mumble` domain.

To check what Mumble sees on a Mac: `mumble labs` lists Labs features as `managed`,
and Mumble's `process.log` names the locks at launch.

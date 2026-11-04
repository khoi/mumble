cask "mumble" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "@URL@"
  name "Mumble"
  desc "Local voice dictation"
  homepage "https://anoma.ly/mumble/"

  livecheck do
    url "https://anoma.ly/release/mumble/desktop/stable.json"
    strategy :json do |json|
      json.dig("release", "version")
    end
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :sequoia

  app "Mumble.app"

  uninstall quit: "anomaly.mumble"

  zap trash: [
    "~/.config/mumble",
    "~/Library/Application Support/mumble",
    "~/Library/Caches/anomaly.mumble",
    "~/Library/HTTPStorages/anomaly.mumble",
    "~/Library/Preferences/anomaly.mumble.plist",
    "~/Library/Saved Application State/anomaly.mumble.savedState",
  ]
end

# Homebrew cask for Idle FighterZ. It lives in the tap repo highskore/homebrew-tap as Casks/idle-fighterz.rb, so:
#   brew install --cask highskore/tap/idle-fighterz
# packaging/bump-cask.sh fills in version and sha256 from a published release.
cask "idle-fighterz" do
  version "0.2.1"
  sha256 "bfbad16748b021c10ddc4d2ff675bb2b47627f3cff7fc4ccd177187bc4a6858a"

  url "https://github.com/highskore/idle-fighterz-widget/releases/download/desktop-v#{version}/IdleFighterZ_#{version}_universal.dmg"
  name "Idle FighterZ"
  desc "Idle Dragon Ball planet game as a desktop widget"
  homepage "https://github.com/highskore/idle-fighterz-widget"

  livecheck do
    url :url
    regex(/^desktop-v(\d+(?:\.\d+)+)$/i)
    strategy :github_latest
  end

  depends_on :macos

  app "Idle FighterZ.app"

  uninstall quit: "com.highskore.idlefighterz"

  zap trash: [
    "~/Library/Application Support/com.highskore.idlefighterz",
    "~/Library/Caches/com.highskore.idlefighterz",
    "~/Library/LaunchAgents/Idle FighterZ.plist",
    "~/Library/Preferences/com.highskore.idlefighterz.plist",
    "~/Library/WebKit/com.highskore.idlefighterz",
  ]
end

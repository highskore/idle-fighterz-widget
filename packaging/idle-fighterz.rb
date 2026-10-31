# Homebrew cask for Idle FighterZ. It lives in the tap repo highskore/homebrew-tap as Casks/idle-fighterz.rb, so:
#   brew install --cask highskore/tap/idle-fighterz
# packaging/bump-cask.sh fills in version and sha256 from a published release.
cask "idle-fighterz" do
  version "0.5.0"
  sha256 "7b3f215d4e46b122f183b8760b11bf61afa947362419cbce6321f075ffb08738"

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

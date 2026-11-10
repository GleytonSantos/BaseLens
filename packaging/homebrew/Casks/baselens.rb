# Homebrew cask (macOS app). Needs the macOS .dmg on the release (the
# workflow's macOS job). For the tap "GleytonSantos/homebrew-baselens":
#   brew install --cask gleytonsantos/baselens/baselens
# scripts/update-channels.sh <version> fills the version and checksum.
cask "baselens" do
  version "0.1.1"
  sha256 :no_check

  url "https://github.com/GleytonSantos/BaseLens/releases/download/v#{version}/BaseLens-#{version}-macos-universal.dmg"
  name "BaseLens"
  desc "Free, private desktop editor for SQLite, PostgreSQL and MySQL"
  homepage "https://github.com/GleytonSantos/BaseLens"

  depends_on macos: ">= :big_sur"

  app "BaseLens.app"

  # the app isn't notarised yet: remove the quarantine flag on install
  postflight do
    system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{appdir}/BaseLens.app"]
  end

  zap trash: [
    "~/Library/Application Support/baselens",
  ]
end

# This repository is also a Homebrew tap. scripts/release.sh updates the version and checksum;
# commit the change after the release is published on GitHub.
cask "asides" do
  version "1.0.3"
  sha256 "08cbf75f40388ecc9c44839050b8327787303582d29f9f80ffd5052622376ffe"

  url "https://github.com/mihaicristianfarcas/asides/releases/download/v#{version}/asides-#{version}.zip"
  name "asides"
  desc "Report bugs out loud, straight into your coding agent's chat"
  homepage "https://asides.mihaicristian.dev/"

  livecheck do
    url :url
    strategy :github_latest
  end

  # asides updates itself (Settings → Updates); `brew upgrade --greedy` updates it too.
  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "asides.app"
  binary "#{appdir}/asides.app/Contents/Helpers/asides"

  uninstall quit: "io.github.mihaicristianfarcas.asides"

  zap trash: "~/Library/Application Support/asides"
end

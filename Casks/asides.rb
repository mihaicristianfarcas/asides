# This repository is also a Homebrew tap. Once a release is published on GitHub, the release workflow
# opens a pull request with its version and checksum.
cask "asides" do
  version "2.0.2"
  sha256 "dd97f2d349824f560d05f736e9b162220943b4754663d99e6c32e720e79f6cd7"

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

# This repository is also a Homebrew tap. Once a release is published on GitHub, the release workflow
# opens a pull request with its version and checksum.
cask "asides" do
  version "2.1.0"
  sha256 "7fe4c6427a3f4c4621c7694d86e8315432a7a0614f9bff1d0734dc60be9d72ad"

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

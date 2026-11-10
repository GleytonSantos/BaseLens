# Homebrew formula (Linux x86_64, prebuilt release). For a tap repository
# "GleytonSantos/homebrew-baselens":  brew install gleytonsantos/baselens/baselens
# scripts/update-channels.sh <version> fills the version and checksum.
class Baselens < Formula
  desc "Free, private desktop editor for SQLite, PostgreSQL and MySQL"
  homepage "https://github.com/GleytonSantos/BaseLens"
  version "0.1.1"
  license "MIT"

  on_linux do
    on_intel do
      url "https://github.com/GleytonSantos/BaseLens/releases/download/v#{version}/baselens-#{version}-linux-x86_64.tar.gz"
      sha256 "705522c24527a33fb3af8f6f4ca98cc18bc1b812346a3fd9473c6629fd5f1c42"
    end
  end

  depends_on "openssl@3"

  def install
    bin.install "baselens"
    share.install "assets" => "baselens"
  end

  test do
    assert_match "baselens", shell_output("#{bin}/baselens --app-id")
  end
end

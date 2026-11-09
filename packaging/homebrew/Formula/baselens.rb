# Homebrew formula (Linux x86_64, prebuilt release). For a tap repository
# "GleytonSantos/homebrew-baselens":  brew install gleytonsantos/baselens/baselens
# scripts/update-channels.sh <version> fills the version and checksum.
class Baselens < Formula
  desc "Free, private desktop editor for SQLite, PostgreSQL and MySQL"
  homepage "https://github.com/GleytonSantos/BaseLens"
  version "0.1.0"
  license "MIT"

  on_linux do
    on_intel do
      url "https://github.com/GleytonSantos/BaseLens/releases/download/v#{version}/baselens-#{version}-linux-x86_64.tar.gz"
      sha256 "610bc3c482ba6b5014b1a0aa008e08116cee4de6bd4f9117ee4965bfe71c9e72"
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

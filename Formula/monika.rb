class Monika < Formula
  desc "Inspectable links between documents, code, and data"
  homepage "https://github.com/KijitoraFinch/Monika"
  version "0.1.0"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/KijitoraFinch/Monika/releases/download/v0.1.0/monika-aarch64-apple-darwin.tar.gz"
      sha256 "567a06d502b2d0306931c20837bb44ec5f6f9227bf58882c9d77f3924ceba870"
    end
    if Hardware::CPU.intel?
      url "https://github.com/KijitoraFinch/Monika/releases/download/v0.1.0/monika-x86_64-apple-darwin.tar.gz"
      sha256 "4bb7314597264f86df813367d98f1c06c2c862db99601174bfc45bf0cdfbe65d"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/KijitoraFinch/Monika/releases/download/v0.1.0/monika-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "cdc46cb3dbcd2156b004fdc8b16ce232029f93345e75241729d1c6571997b604"
    end
    if Hardware::CPU.intel?
      url "https://github.com/KijitoraFinch/Monika/releases/download/v0.1.0/monika-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "b8a442857f18d0b12f6b7bbc67c75832986f0538a851a24eb27444d1f7ec3e2d"
    end
  end
  license "MIT"
  depends_on "jq"

  BINARY_ALIASES = {
    "aarch64-apple-darwin": {},
    "aarch64-unknown-linux-gnu": {},
    "x86_64-apple-darwin": {},
    "x86_64-unknown-linux-gnu": {}
  }

  def target_triple
    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"

    "#{cpu}-#{os}"
  end

  def install_binary_aliases!
    BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
  end

  def install
    if OS.mac? && Hardware::CPU.arm?
      bin.install "monika"
    end
    if OS.mac? && Hardware::CPU.intel?
      bin.install "monika"
    end
    if OS.linux? && Hardware::CPU.arm?
      bin.install "monika"
    end
    if OS.linux? && Hardware::CPU.intel?
      bin.install "monika"
    end

    install_binary_aliases!

    # Homebrew will automatically install these, so we don't need to do that
    doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
    leftover_contents = Dir["*"] - doc_files

    # Install any leftover files in pkgshare; these are probably config or
    # sample files.
    pkgshare.install(*leftover_contents) unless leftover_contents.empty?
  end
end

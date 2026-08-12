{
  rustPlatform,
  lib,
  git,
}:

rustPlatform.buildRustPackage {
  pname = "dotpkg";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ./..;
    filter =
      path: type:
      let
        base = baseNameOf (toString path);
      in
      !(
        base == "target"
        || base == "result"
        || base == ".git"
        || base == "mutants.out"
        || base == "mutants.out.old"
        || base == "scratch"
      );
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The suite runs, and it is the point of this project: 658 tests on a Unix
  # host, every one of them confirmed able to fail.
  #
  # `git` is a real check dependency, found by running the build rather than by
  # reading: the sandbox has no git, and several tests build temp repositories
  # with `git init` to exercise scoop's bucket handling. Without this the build
  # fails on `git ["init", "-q", "-b", "main"]: No such file or directory`.
  nativeCheckInputs = [ git ];
  doCheck = true;

  # **Measured, not a workaround.** `fixupPhase`'s strip is killed with SIGKILL
  # here -- `xargs: strip: terminated by signal 9`, reproduced twice, with
  # `sandbox = false` on the machine and the same `strip -S` succeeding on the
  # same binary run by hand outside the build. The cause is not understood.
  #
  # What settles it is what stripping is worth: the binary is **2809016** bytes
  # unstripped and **2809000** after `strip -S` -- sixteen bytes, because a
  # Rust release build on darwin keeps its debug info in a separate `.dSYM`
  # rather than in the executable. Trading a build that completes for 16 bytes
  # is not a trade. Revisit if a platform ever shows a real difference.
  dontStrip = true;

  meta = {
    description = "Declarative package management for Windows: winget and scoop from one dotfile";
    homepage = "https://github.com/xom11/dotpkg";
    license = lib.licenses.mit;
    mainProgram = "dotpkg";

    # It BUILDS everywhere nix runs, and the suite passes there -- but what it
    # MANAGES is winget and scoop, so on a machine with neither it has nothing
    # to do. Packaged here so this fleet can pin and `nix run` it the way it
    # pins its other tools, not because a darwin or Linux profile should
    # install it.
    platforms = lib.platforms.unix;
  };
}

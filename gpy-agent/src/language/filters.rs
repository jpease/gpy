//! Linguist exclusion globs for the language-detection walk.
//!
//! Glob data ported from `hyperpolyglot` 0.1.7 / `github/linguist`
//! `vendor.yml` (both MIT). Kept for parity: excluded files would otherwise
//! enter `confidence_for`'s denominator and shift every reported ratio.

use ignore::overrides::{Override, OverrideBuilder};
use std::path::Path;

/// Vendored-code exclusions (bundled deps, generated scaffolding, third-party JS).
const VENDOR_GLOBS: &[&str] = &[
    "!**/cache/**",
    "!**/[Dd]ependencies/**",
    "!**/dist/**",
    "!**/deps/**",
    "!**/configure",
    "!**/config.guess",
    "!**/config.sub",
    "!**/aclocal.m4",
    "!**/libtool.m4",
    "!**/ltoptions.m4",
    "!**/ltsugar.m4",
    "!**/ltversion.m4",
    "!**/lt~obsolete.m4",
    "!**/dotnet-install.ps1",
    "!**/dotnet-install.sh",
    "!**/cpplint.py",
    "!**/node_modules/**",
    "!**/.yarn/releases/**",
    "!**/_esy",
    "!**/bower_components/**",
    "!**/rebar/**",
    "!**/erlang.mk",
    "!**/Godeps/_workspace/**",
    "!**/testdata/**",
    "!**/.indent.pro",
    "!**/*[.-]min.{css,js}",
    "!**/*import.{css,less,scss,styl}",
    "!**/bootstrap*.{js,css,less,scss,styl}",
    "!**/custom.bootstrap*.{js,css,less,scss,styl}",
    "!**/font*awesome.{js,css,less,scss,styl}",
    "!**/font*awesome/**/*.{js,css,less,scss,styl}",
    "!**/foundation.{css,less,scss,styl}",
    "!**/normalize.{css,less,scss,styl}",
    "!**/skeleton.{css,less,scss,styl}",
    "!**/[Bb]ourbon.{css,less,scss,styl}",
    "!**/animate.{css,less,scss,styl}",
    "!**/materialize.{css,less,scss,styl,js}",
    "!**/select2/*.{css,scss,js}",
    "!**/bulma.{css,sass,scss}",
    "!**/{3rd,[Tt]hird}*[Pp]arty/**",
    "!**/vendor/**",
    "!**/vendors/**",
    "!**/external/**",
    "!**/extern/**",
    "!**/[Vv]endor/**",
    "!**/debian/**",
    // Haxelib projects often contain a neko bytecode file named run.n
    "!**/run.n",
    "!**/bootstrap-datepicker/**",
    "!**/jquery*.{js,css}",
    "!**/fuelux.js",
    "!**/bootbox.js",
    "!**/pdf.worker.js",
    "!**/slick.*.js",
    "!**/Leaflet.Coordinates*.src.js",
    "!**/leaflet.draw-src.js",
    "!**/leaflet.draw.css",
    "!**/Control.FullScreen.{css,js}",
    "!**/leaflet.spin.js",
    "!**/wicket-leaflet.js",
    "!**/.sublime-project",
    "!**/.sublime-workspace",
    "!**/.vscode",
    "!**/prototype*.js",
    "!**/effects.js",
    "!**/controls.js",
    "!**/dragdrop.js",
    "!**/*.d.ts",
    "!**/mootools*.js",
    "!**/dojo.js",
    "!**/MochiKit.js",
    "!**/yahoo-*.js",
    "!**/yui*.js",
    "!**/ckeditor.js",
    "!**/tiny_mce*.js",
    "!**/tiny_mce/{langs,plugins,themes,utils}/**",
    "!**/ace-build/**",
    "!**/fontello*.css",
    "!**/MathJax/**",
    "!**/Chart.js",
    "!**/[Cc]ode[Mm]irror/*{lib,mode,theme,addon,keymap,demo}/**",
    "!**/shBrush*.js",
    "!**/shCore.js",
    "!**/shLegacy.js",
    "!**/angular*.js",
    "!**/d3*.js",
    "!**/react*.js",
    "!**/flow-typed/*.js",
    "!**/modernizr*.js",
    "!**/knockout-*.js",
    "!**/{doc,docs}/{build,themes,theme,templates,template,static}",
    "!**/admin_media/**",
    "!**/env/**",
    "!**/fabfile.py",
    "!**/waf",
    "!**/.osx",
    "!**/.xctemplate/**",
    "!**/.imageset/**",
    "!**/Carthage/**",
    "!**/Sparkle/**",
    "!**/Crashlytics.framework/**",
    "!**/Fabric.framework/**",
    "!**/BuddyBuildSDK.framework/**",
    "!**/Realm.framework",
    "!**/RealmSwift.framework",
    "!**/.gitattributes",
    "!**/.gitignore",
    "!**/.gitmodules",
    "!**/gradlew",
    "!**/gradlew.bat",
    "!**/gradle/wrapper/**",
    "!**/mvnw",
    "!**/mvnw.cmd",
    "!**/.mvn/wrapper/**",
    "!**/*-vsdoc.js",
    "!**/*.intellisense.js",
    // jQuery validation plugin (MS bundles this with asp.net mvc)
    "!**/jquery*.validate*.js",
    "!**/jquery*.unobtrusive-ajax.js.js",
    "!**/[Mm]icrosoft{[Aa]jax,[Vv]alidation}*.js",
    "!**/[Pp]ackages/*.*/**",
    "!**/extjs/*.{js,xml,txt,html,properties}",
    "!**/extjs/{.sencha,docs,builds,cmd,examples,locale,packages,plugins,resources,src,welcome}/**",
    "!**/html5shiv.js",
    "!**/[Tt]es{t,ts}/fixtures/**",
    "!**/[Ss]pe{c,cs}/fixtures/**",
    "!**/cordova*.js",
    "!**/foundation*.js",
    "!**/Vagrantfile",
    "!**/.[Dd][Ss]_[Ss]tore",
    "!**/vignettes/**",
    "!**/inst/extdata/**",
    "!**/octicons.css",
    "!**/sprockets-octicons.scss",
    "!**/activiator",
    "!**/activator.bat",
    "!**/proguard.pro",
    "!**/proguard-rules.pro",
    "!**/puphet/**",
    "!**/.google_apis/**",
    "!**/Jenkinsfile",
];

const DOCUMENTATION_GLOBS: &[&str] = &[
    "!**/[Dd]ocs/**",
    "!**/[Dd]oc/**",
    "!**/[Dd]ocumentation/**",
    "!**/[Gg]roovydoc/**",
    "!**/[Jj]avadoc/**",
    "!**/[Mm]an/**",
    "!**/[Ee]xamples/**",
    "!**/[Dd]emo/**",
    "!**/[Dd]emos/**",
    "!**/inst/doc/**",
    "!**/CHANGE*",
    "!**/CHANGES*",
    "!**/CHANGELOG*",
    "!**/CONTRIBUTING*",
    "!**/COPYING*",
    "!**/INSTALL*",
    "!**/LICEN[CS]E*",
    "!**/[Ll]icen[cs]e*",
    "!**/README*",
    "!**/[Rr]eadme*",
    "!**/[Ss]ample/**",
    "!**/[Ss]amples/**",
];

/// Builds the exclusion [`Override`] rooted at `root`. Globs are validated by
/// tests; failure arms degrade to an unfiltered walk rather than abort the daemon.
pub(super) fn exclusion_overrides(root: &Path) -> Override {
    let mut builder = OverrideBuilder::new(root);
    for glob in DOCUMENTATION_GLOBS.iter().chain(VENDOR_GLOBS) {
        if builder.add(glob).is_err() {
            return Override::empty();
        }
    }
    builder.build().unwrap_or_else(|_| Override::empty())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn every_glob_is_accepted() {
        let mut builder = OverrideBuilder::new("./");
        for glob in DOCUMENTATION_GLOBS.iter().chain(VENDOR_GLOBS) {
            assert!(builder.add(glob).is_ok(), "malformed glob: {glob}");
        }
        assert!(builder.build().is_ok(), "override set must build");
    }

    #[test]
    fn vendored_and_documentation_paths_are_ignored() {
        let overrides = exclusion_overrides(Path::new("./"));
        for path in [
            "vendor/bundled.rs",
            "node_modules/pkg/index.js",
            "third_party/lib.c",
            "README.md",
            "docs/guide.md",
        ] {
            assert!(
                overrides.matched(path, false).is_ignore(),
                "{path} should be excluded"
            );
        }
    }

    #[test]
    fn ordinary_source_paths_are_not_ignored() {
        let overrides = exclusion_overrides(Path::new("./"));
        for path in ["src/main.rs", "lib/app.py", "cmd/server/main.go"] {
            assert!(
                overrides.matched(path, false).is_none(),
                "{path} should not be excluded"
            );
        }
    }
}

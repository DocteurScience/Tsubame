project(
  'tsubame',
  version: '0.1.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.0',
)

i18n = import('i18n')
gnome = import('gnome')

application_id = 'io.github.DocteurScience.Tsubame'

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')

subdir('data')
subdir('po')

# Cargo construit le binaire ; Meson l'installe avec les données.
cargo = find_program('cargo')
cargo_target_dir = meson.project_build_root() / 'target'
if get_option('buildtype') == 'release'
  cargo_profile = ['--release']
  rust_target = 'release'
else
  cargo_profile = []
  rust_target = 'debug'
endif

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  console: true,
  output: meson.project_name(),
  install: true,
  install_dir: bindir,
  env: {'TSUBAME_LOCALEDIR': localedir},
  command: [
    'sh', '-c',
    '"$1" build --manifest-path "$2" --target-dir "$3" $4 && cp "$3/$5/tsubame" "$6"',
    'cargo-build',
    cargo,
    meson.project_source_root() / 'Cargo.toml',
    cargo_target_dir,
    ' '.join(cargo_profile),
    rust_target,
    '@OUTPUT@',
  ],
)

gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)

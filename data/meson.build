desktop_file = i18n.merge_file(
  input: application_id + '.desktop.in',
  output: application_id + '.desktop',
  type: 'desktop',
  po_dir: '../po',
  install: true,
  install_dir: datadir / 'applications',
)

metainfo_file = i18n.merge_file(
  input: application_id + '.metainfo.xml.in',
  output: application_id + '.metainfo.xml',
  po_dir: '../po',
  install: true,
  install_dir: datadir / 'metainfo',
)

desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('Valider le fichier desktop', desktop_file_validate, args: [desktop_file])
endif

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'Valider les métadonnées AppStream',
    appstreamcli,
    args: ['validate', '--no-net', '--explain', metainfo_file],
  )
endif

install_data(
  'icons/hicolor/scalable/apps' / application_id + '.svg',
  install_dir: datadir / 'icons/hicolor/scalable/apps',
)
install_data(
  'icons/hicolor/symbolic/apps' / application_id + '-symbolic.svg',
  install_dir: datadir / 'icons/hicolor/symbolic/apps',
)

install_data(
  application_id + '.gschema.xml',
  install_dir: datadir / 'glib-2.0/schemas',
)
glib_compile_schemas = find_program('glib-compile-schemas', required: false)
if glib_compile_schemas.found()
  test(
    'Valider le schéma GSettings',
    glib_compile_schemas,
    args: ['--strict', '--dry-run', meson.current_source_dir()],
  )
endif

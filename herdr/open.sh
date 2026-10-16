#!/bin/sh
# Abre el visor como popup sobre el panel activo, en la carpeta de ese panel.
H="${HERDR_BIN_PATH:-herdr}"
exec "$H" plugin pane open --plugin local.gitview --entrypoint viewer --placement popup \
  --width 94% --height 92% --env "HERDR_PLUGIN_CONTEXT_JSON=${HERDR_PLUGIN_CONTEXT_JSON:-}"

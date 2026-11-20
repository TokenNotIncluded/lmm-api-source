/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
/*
Copyright (C) 2026 LIghtJUNction
*/
export const PI_INSTALL_LATEST_COMMAND =
  'npm exec --yes --prefer-online --package=@tokennotincluded/pi-lmm-provider@0.1.0-alpha.7 -- lmm-pi-provider npm:@tokennotincluded/pi-lmm-provider@0.1.0-alpha.7'

export const DSH_PACKAGE_LATEST =
  '@tokennotincluded/dsh-lmm-provider@0.1.0-alpha.4'

const DSH_WEB_RELEASE_URL =
  'https://github.com/TokenNotIncluded/dsh-lmm-provider/releases/download/v0.1.0-alpha.9/dsh-lmm-provider.tgz'

export const DSH_WEB_INSTALL_PORTABLE_COMMAND = `dsh plugin --profile web add "${DSH_WEB_RELEASE_URL}" --ignore-scripts`

export const DSH_WEB_INSTALL_LATEST_COMMAND = DSH_WEB_INSTALL_PORTABLE_COMMAND

export const DSH_WEB_INSTALL_LATEST_WINDOWS_COMMAND = `dsh.cmd plugin --profile web add "${DSH_WEB_RELEASE_URL}" --ignore-scripts`

export const DSH_DOWNLOAD_LATEST_COMMAND = `npm pack "${DSH_WEB_RELEASE_URL}" --ignore-scripts`

export const CODEWHALE_UNIX_INSTALL_COMMAND =
  'curl -fsSL https://codewhale.net/install.sh | sh'

export const CODEWHALE_NPM_INSTALL_COMMAND = 'npm install -g codewhale@latest'

export const CODEWHALE_CLONE_COMMAND =
  'git clone --branch v0.1.0-alpha.4 --depth 1 https://github.com/TokenNotIncluded/codewhale-lmm-provider.git'

export const CODEWHALE_ADAPTER_INSTALL_COMMAND =
  'cd codewhale-lmm-provider && npm install --global .'

export const CODEWHALE_LOGIN_COMMAND = 'codewhale-lmm login'
export const CODEWHALE_MODELS_COMMAND = 'codewhale-lmm models'
export const CODEWHALE_RUN_COMMAND =
  "codewhale-lmm run --model '<FULL_MODEL_ID>'"
export const CODEWHALE_INSTALL_COMMAND =
  'git clone --branch v0.1.0-alpha.4 --depth 1 https://github.com/TokenNotIncluded/codewhale-lmm-provider.git && cd codewhale-lmm-provider && npm install --global .'

const assert = require('node:assert/strict');
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const vscode = require('vscode');
const manifest = require('./package.json');

exports.run = async () => {
  const stage = name => fs.writeFileSync(path.join(__dirname, '../stage'), name + '\n');
  stage('starting extension activation');
  const { route, progressPort } = JSON.parse(fs.readFileSync(path.join(__dirname, '../fixture.json')));
  const extension = vscode.extensions.getExtension(`${manifest.publisher}.${manifest.name}`);
  assert.ok(extension);
  await extension.activate();
  assert.ok(extension.isActive);
  const [{ vendor }] = manifest.contributes.languageModelChatProviders;
  stage('discovering the granted model');
  const models = await vscode.lm.selectChatModels({ vendor });
  const model = models.find(candidate => candidate.id === route.id);
  assert.ok(model, 'actual VS Code model selection must return the authorized Core route');
  assert.equal(model.family, route.upstream_model);
  const cancellation = new vscode.CancellationTokenSource();
  try {
    const invoke = prompt => model.sendRequest([vscode.LanguageModelChatMessage.User(prompt)], {}, cancellation.token);
    stage('waiting for the first live model chunk');
    const response = await invoke('stream-pause');
    let text = '';
    for await (const chunk of response.text) {
      if (!text && chunk) {
        stage('received text before Core released the terminal frame');
        await new Promise((resolve, reject) => {
          const socket = net.connect(progressPort, '127.0.0.1');
          socket.once('error', reject);
          socket.once('connect', () => { socket.end(); resolve(); });
        });
      }
      text += chunk;
    }
    assert.equal(text, 'local stream answer');
    for (const prompt of ['rate-limit', 'confirmed-rate-limit']) {
      stage('checking ' + prompt);
      let errorText = '';
      await assert.rejects(async () => {
        const refused = await invoke(prompt);
        for await (const chunk of refused.text) errorText += chunk;
      }, /HTTP 502/);
      assert.equal(errorText, '', 'supplier refusal must not produce assistant text');
    }
    stage('verified terminal text and HTTP error');
    console.log('LMM_AUTHENTICATED_VSIX_CORE_PASS');
  } finally {
    cancellation.dispose();
  }
};

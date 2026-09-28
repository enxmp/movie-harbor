const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  for (const platform of ['windows', 'macos']) {
    const page = await browser.newPage({ viewport: { width: 1180, height: 900 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(platform => {
      window.calls = [];
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
        transformCallback: fn => fn,
        invoke: async (cmd, args) => {
          window.calls.push({ cmd, args });
          if (cmd === 'plugin:event|listen') return 1;
          if (cmd === 'environment') return { platform, apple: platform === 'macos', nvenc: platform === 'windows', ffmpeg: 'ffmpeg', ffprobe: 'ffprobe' };
          if (cmd === 'plugin:dialog|open') return args.options.directory ? '/shared-media' : '/selected-key';
          if (cmd === 'enable_remote') return { token: 'a'.repeat(64), root: '/shared-media' };
          if (cmd === 'remote_connect') return { platform: 'macos', apple: true, nvenc: false, ffmpeg: 'ffmpeg', ffprobe: 'ffprobe' };
          if (cmd === 'remote_call' && args.action === 'queue') return [];
        },
      };
    }, platform);
    await page.goto('http://127.0.0.1:1420');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByText('Remote conversion', { exact: true }).click();
    if (await page.evaluate(() => window.calls.some(c => c.cmd === 'enable_remote'))) throw Error('Hosting started automatically');
    if (platform === 'macos') {
      await page.getByRole('button', { name: 'Enable remote control…', exact: true }).click();
      const token = page.getByLabel('Access token', { exact: true });
      await token.waitFor();
      if (await token.getAttribute('type') !== 'password') throw Error('Token is not masked');
      await page.getByRole('button', { name: 'Show token', exact: true }).click();
      if (await token.inputValue() !== 'a'.repeat(64)) throw Error('Token mismatch');
      await page.getByRole('button', { name: 'Disable remote control', exact: true }).click();
      await page.getByRole('button', { name: 'Enable remote control…', exact: true }).waitFor();
    } else {
      await page.getByLabel('Mac hostname', { exact: true }).fill('mac.local');
      await page.getByLabel('SSH username', { exact: true }).fill('example');
      await page.locator('.remote-settings').getByRole('button', { name: 'Browse', exact: true }).nth(0).click();
      await page.locator('.remote-settings').getByRole('button', { name: 'Browse', exact: true }).nth(1).click();
      await page.getByLabel('Mac access token', { exact: true }).fill('a'.repeat(64));
      await page.getByRole('button', { name: 'Connect to Mac', exact: true }).click();
      await page.getByRole('button', { name: 'Disconnect Mac', exact: true }).waitFor();
      const config = await page.evaluate(() => window.calls.find(c => c.cmd === 'remote_connect').args.config);
      if (config.host !== 'mac.local' || config.user !== 'example' || config.local_root !== '/shared-media' || config.key_path !== '/selected-key') throw Error('Incorrect connection configuration');
      await page.getByRole('button', { name: 'Disconnect Mac', exact: true }).click();
      await page.getByRole('button', { name: 'Connect to Mac', exact: true }).waitFor();
    }
    if (errors.length) throw Error(errors.join('\n'));
    await page.close();
  }
  await browser.close();
  console.log('Remote settings passed on Windows and Mac layouts; IPC mocked.');
})().catch(error => { console.error(error); process.exit(1); });

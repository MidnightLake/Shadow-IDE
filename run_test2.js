import { chromium } from 'playwright';

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on('console', msg => console.log('PAGE LOG:', msg.text()));
  page.on('pageerror', error => console.log('PAGE ERROR:', error.message));
  
  await page.goto('http://127.0.0.1:5173');
  await page.waitForTimeout(3000);
  
  await page.screenshot({ path: 'screenshot.png' });
  console.log('Screenshot saved to screenshot.png');
  
  await browser.close();
})();

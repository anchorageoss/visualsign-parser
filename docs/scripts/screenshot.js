#!/usr/bin/env node
/**
 * Screenshot tool for validating Mintlify pages
 * Usage: node scripts/screenshot.js <page-path> [output-file] [scroll-offset]
 * Example: node scripts/screenshot.js /getting-started
 * Example: node scripts/screenshot.js /getting-started screenshot.png 400
 *
 * Output files default to ./screenshots/ directory
 */

const puppeteer = require('/tmp/node_modules/puppeteer');
const path = require('path');
const fs = require('fs');

const CHROME_PATH = '/home/user/.cache/puppeteer/chrome/linux-144.0.7559.96/chrome-linux64/chrome';
const BASE_URL = 'http://localhost:3000';
const SCREENSHOTS_DIR = path.join(__dirname, '..', 'screenshots');

// Ensure screenshots directory exists
if (!fs.existsSync(SCREENSHOTS_DIR)) {
  fs.mkdirSync(SCREENSHOTS_DIR, { recursive: true });
}

async function screenshot(pagePath, outputFile, scrollOffset = 0) {
  // If outputFile is not absolute, put it in screenshots dir
  if (!path.isAbsolute(outputFile)) {
    outputFile = path.join(SCREENSHOTS_DIR, outputFile);
  }
  const browser = await puppeteer.launch({
    executablePath: CHROME_PATH,
    headless: true,
    args: ['--no-sandbox']
  });

  const page = await browser.newPage();
  await page.setViewport({ width: 1200, height: 900 });
  await page.goto(`${BASE_URL}${pagePath}`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('.mermaid', { timeout: 5000 }).catch(() => {});
  await new Promise(r => setTimeout(r, 1500));

  if (scrollOffset > 0) {
    await page.evaluate((offset) => window.scrollBy(0, offset), scrollOffset);
    await new Promise(r => setTimeout(r, 500));
  }

  await page.screenshot({ path: outputFile, fullPage: false });
  await browser.close();

  console.log(`Screenshot saved to ${outputFile}`);
}

const [,, pagePath, outputFileArg, scrollOffset] = process.argv;

if (!pagePath) {
  console.error('Usage: node scripts/screenshot.js <page-path> [output-file] [scroll-offset]');
  console.error('Example: node scripts/screenshot.js /getting-started');
  console.error('Example: node scripts/screenshot.js /getting-started shot.png 400');
  process.exit(1);
}

// Default filename based on page path
const defaultFilename = pagePath.replace(/^\//, '').replace(/\//g, '-') + '.png' || 'screenshot.png';
const outputFile = outputFileArg || defaultFilename;

screenshot(pagePath, outputFile, parseInt(scrollOffset) || 0);

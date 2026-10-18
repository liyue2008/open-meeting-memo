// Preserve every production npm package's license text in the offline bundle.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));
const projectNotice = JSON.parse(
  fs.readFileSync(
    new URL('../licenses/react-remove-scroll-bar-2.3.8.notice.json', import.meta.url),
    'utf8',
  ),
);
const projectLicense = new URL(
  '../licenses/react-remove-scroll-bar-project-MIT.LICENSE',
  import.meta.url,
);

// The 2.3.8 tarball omits its MIT text. This one reviewed upstream-project
// mapping is not a general fallback for packages that only declare a license.
export function verifyProjectNotice(location, entry, metadata, text) {
  const repository =
    typeof metadata.repository === 'string' ? metadata.repository : metadata.repository?.url;
  if (
    location !== 'node_modules/react-remove-scroll-bar' ||
    projectNotice.version !== '2.3.8' ||
    metadata.name !== projectNotice.name ||
    metadata.version !== projectNotice.version ||
    entry.version !== projectNotice.version ||
    entry.integrity !== projectNotice.integrity ||
    entry.license !== projectNotice.license ||
    metadata.license !== projectNotice.license ||
    repository !== projectNotice.repository ||
    Buffer.byteLength(text) !== projectNotice.noticeBytes ||
    crypto.createHash('sha256').update(text).digest('hex') !== projectNotice.noticeSha256
  ) {
    throw new Error(`Unverified missing-license notice: ${location}`);
  }
  return text;
}
function collectNotices() {
  const lock = JSON.parse(fs.readFileSync(path.join(root, 'package-lock.json'), 'utf8'));
  let output =
    'Open Meeting Memo desktop — production npm dependency notices\n\nGenerated from package-lock.json and installed package license files.\nRadix palette attribution is separately preserved in RADIX-COLORS-LICENSE.txt.\n\n';
  for (const [location, entry] of Object.entries(lock.packages).sort(([a], [b]) =>
    a.localeCompare(b, 'en'),
  )) {
    if (!location || entry.dev) continue;
    if (!location.startsWith('node_modules/') || entry.link)
      throw new Error(`Unexpected package location: ${location}`);
    const directory = path.join(root, location);
    const metadata = JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8'));
    if (metadata.version !== entry.version) throw new Error(`Version drift: ${location}`);
    const licenses = fs
      .readdirSync(directory)
      .filter(
        (name) =>
          /^(license|copying|notice)(?:[.-]|$)/i.test(name) &&
          fs.statSync(path.join(directory, name)).isFile(),
      )
      .sort();
    output += `${'='.repeat(72)}\n${metadata.name}@${metadata.version}\nLicense: ${entry.license ?? metadata.license ?? 'See text below'}\n\n`;
    if (!licenses.length) {
      const text = verifyProjectNotice(
        location,
        entry,
        metadata,
        fs.readFileSync(projectLicense, 'utf8'),
      );
      output += `Notice source: ${projectNotice.noticeSourceUrl}\nSource SHA-256: ${projectNotice.noticeSha256}\nScope: ${projectNotice.scope}\n\n--- react-remove-scroll-bar-project-MIT.LICENSE ---\n${text.trimEnd()}\n\n`;
    }
    for (const name of licenses)
      output += `--- ${name} ---\n${fs.readFileSync(path.join(directory, name), 'utf8').trimEnd()}\n\n`;
  }
  output = output.replace(/\r\n/g, '\n').trimEnd() + '\n';
  const destination = path.join(root, 'public/THIRD-PARTY-NOTICES.txt');
  if (process.argv.includes('--check')) {
    if (fs.readFileSync(destination, 'utf8') !== output)
      throw new Error('Production dependency notices drift');
  } else fs.writeFileSync(destination, output);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  collectNotices();

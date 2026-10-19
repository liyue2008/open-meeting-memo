// Preserve every production npm package's shipped license text in the offline bundle.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));
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
  if (!licenses.length) throw new Error(`Missing license text: ${location}`);
  output += `${'='.repeat(72)}\n${metadata.name}@${metadata.version}\nLicense: ${entry.license ?? metadata.license ?? 'See text below'}\n\n`;
  for (const name of licenses)
    output += `--- ${name} ---\n${fs.readFileSync(path.join(directory, name), 'utf8').trimEnd()}\n\n`;
}
output = output.trimEnd() + '\n';
const destination = path.join(root, 'public/THIRD-PARTY-NOTICES.txt');
if (process.argv.includes('--check')) {
  if (fs.readFileSync(destination, 'utf8') !== output)
    throw new Error('Production dependency notices drift');
} else fs.writeFileSync(destination, output);

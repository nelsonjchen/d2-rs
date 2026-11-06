// SPDX-License-Identifier: MPL-2.0
// These targets and expected bytes come from original Go, not native output.
export async function checkTargetAscii(client, fixture) {
  const cases = fixture.cases.filter(c => c.options?.ascii === true);
  if (cases.length !== 8) throw new Error("Incomplete ASCII client fixture");
  for (const c of cases) {
    const actual = await client.render(structuredClone(c.diagram), c.options);
    if (actual !== c.output) throw new Error(`${c.name}: deserialized target ASCII differs`);
  }
  const compiled = await client.compile("a");
  for (const c of cases) {
    // Keep the real compile metadata while replacing the visible target. A
    // source replay would paint the original "a" instead of these records.
    compiled.diagram.shapes = structuredClone(c.diagram.shapes || []);
    compiled.diagram.connections = structuredClone(c.diagram.connections || []);
    const actual = await client.render(compiled.diagram, c.options);
    if (actual !== c.output) throw new Error(`${c.name}: mutated target ASCII differs`);
  }
  return cases.length * 2;
}

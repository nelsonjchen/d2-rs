// SPDX-License-Identifier: MPL-2.0

export async function checkUrlCodec(api, fixture) {
  // Pinned lib/urlenc.Encode uses Go BestCompression and padded URL base64.
  for (const [source, encoded] of [["", "AwA="], ["a -> b", "SlTQtVNIAgwA"],
    ["雪 e\u0301 مرحبا", "ejl7lULqmUaFm603Nt5Ye2PFjeWAAQA="], ["\0\n\r\t", "YuDi5QQMAA=="]]) {
    if (await api.encode(source) !== encoded) throw new Error("Encoded bytes differ from Go");
    if (await api.decode(encoded) !== source) throw new Error("Decoded Go encoding differs");
  }
  if (fixture.cases.length !== 987) throw new Error("Incomplete source codec inventory");
  for (const row of fixture.cases) {
    let result, error;
    try { result = await api.decode(row.encoded); }
    catch (thrown) { error = thrown instanceof Error ? thrown.message : String(thrown); }
    if (row.expected.error) {
      if (error !== row.expected.error.message) throw new Error(`${row.name}: ${JSON.stringify({ error, expected: row.expected.error.message, result })}`);
    } else if (error !== undefined || result !== row.expected.result) {
      throw new Error(`${row.name}: decoded content differs from Go`);
    }
    if (await api.decode("AwA=") !== "") throw new Error(`${row.name}: unhealthy followup`);
  }
  // Exercise larger scripts as well as the exact Go encoding contracts.
  for (const source of ["", "a -> b\n", "é😀\n", "\u2028\u2029", "repeat\n".repeat(10000)]) {
    if (await api.decode(await api.encode(source)) !== source) throw new Error("Codec roundtrip failed");
  }
  return fixture.cases.length;
}

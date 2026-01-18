export function unpackSingleBoolean(result) {
  if (!result) {
    throw new Error('DBus result is required');
  }

  const unpacked = result.deep_unpack();
  if (!Array.isArray(unpacked)) {
    throw new Error(`DBus result must unpack to array, got ${typeof unpacked}`);
  }
  if (unpacked.length !== 1) {
    throw new Error(`DBus result must have 1 element, got ${unpacked.length}`);
  }

  const [value] = unpacked;
  if (typeof value !== 'boolean') {
    throw new Error(`DBus result must be boolean, got ${typeof value}`);
  }

  return value;
}

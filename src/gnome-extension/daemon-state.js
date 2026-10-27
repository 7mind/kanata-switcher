const EMPTY_LAYER = '';
const SOURCE_EXTERNAL = 'external';
const SOURCE_FOCUS = 'focus';

export function isDaemonOwnerAvailable(owner) {
  if (owner === null || owner === undefined) {
    return false;
  }
  if (typeof owner !== 'string') {
    throw new Error(`Daemon owner must be string, got ${typeof owner}`);
  }
  if (owner.trim().length === 0) {
    return false;
  }
  return true;
}

function cloneStatus(status, context) {
  if (!status || typeof status !== 'object') {
    throw new Error(`${context} must be an object`);
  }
  if (typeof status.layer !== 'string') {
    throw new Error(`${context}.layer must be a string`);
  }
  if (!Array.isArray(status.virtualKeys)) {
    throw new Error(`${context}.virtualKeys must be an array`);
  }
  for (const virtualKey of status.virtualKeys) {
    if (typeof virtualKey !== 'string') {
      throw new Error(`${context}.virtualKeys entries must be strings`);
    }
  }
  if (typeof status.source !== 'string') {
    throw new Error(`${context}.source must be a string`);
  }
  return {
    layer: status.layer,
    virtualKeys: [...status.virtualKeys],
    source: status.source
  };
}

export function initialStatusState() {
  return {
    layer: EMPTY_LAYER,
    virtualKeys: [],
    source: SOURCE_EXTERNAL
  };
}

export function initialFocusStatusState() {
  return {
    layer: EMPTY_LAYER,
    virtualKeys: [],
    source: SOURCE_FOCUS
  };
}

export function disconnectedState(lastStatus, focusStatus) {
  const status = cloneStatus(lastStatus, 'lastStatus');
  const nextFocusStatus = cloneStatus(focusStatus, 'focusStatus');
  return {
    status,
    focusStatus: nextFocusStatus,
    lastStatus: status,
    paused: false
  };
}

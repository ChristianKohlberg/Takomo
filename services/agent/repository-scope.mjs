// Scope is host-owned input. Tools may narrow it, but cannot expand it.
export function repositoryPath(value, { root = false } = {}) {
  if (root && value === '.') return value;
  if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 1024
      || value.startsWith('/') || /^[A-Za-z]:/.test(value) || /[\\\x00-\x1f\x7f]/u.test(value)
      || value.split('/').some(part => !part || part === '.' || part === '..')) {
    throw new Error('Use a literal repository-relative file or directory path, without empty segments, backslashes or traversal. Use "." explicitly for the whole repository.');
  }
  return value;
}
export const underPath = (path, prefix) => prefix === '.' || path === prefix || path.startsWith(`${prefix}/`);

export function repositoryScope(value) {
  if (value === undefined) return { include: ['.'], exclude: [] };
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).some(key => !['include', 'exclude'].includes(key))) {
    throw new Error('Repository scope must contain include and optional exclude path arrays.');
  }
  const paths = (items, name, minimum) => {
    if (!Array.isArray(items) || items.length < minimum || items.length > 64) throw new Error(`scope.${name} must contain ${minimum}–64 literal paths.`);
    const unique = [...new Set(items.map(path => repositoryPath(path, { root: true })))].sort();
    return unique.filter(path => !unique.some(parent => parent !== path && underPath(path, parent)));
  };
  return { include: paths(value.include, 'include', 1), exclude: paths(value.exclude === undefined ? [] : value.exclude, 'exclude', 0) };
}

export function repositoryLimits(value = {}) {
  const defaults = { max_files: 100_000, max_source_bytes: 1_000_000_000, max_tool_calls: 100 };
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).some(key => !Object.hasOwn(defaults, key))) throw new Error('Unsupported repository limits.');
  return Object.fromEntries(Object.entries(defaults).map(([key, maximum]) => {
    const n = value[key] ?? maximum;
    if (!Number.isSafeInteger(n) || n < 1 || n > maximum) throw new Error(`${key} must be an integer from 1 to ${maximum}.`);
    return [key, n];
  }));
}

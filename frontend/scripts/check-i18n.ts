import { readdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

type JsonObject = { [key: string]: JsonValue }
type JsonValue = string | number | boolean | null | JsonValue[] | JsonObject

const LOCALES_DIR = fileURLToPath(new URL('../src/i18n/locales', import.meta.url))
const BASELINE_LOCALE = 'zh-CN'
const COMPARED_LOCALE = 'en-US'

async function listNamespaces(locale: string): Promise<string[]> {
  const entries = await readdir(join(LOCALES_DIR, locale), {
    withFileTypes: true,
  })
  return entries
    .filter((entry) => entry.isFile() && entry.name.endsWith('.json'))
    .map((entry) => entry.name.replace(/\.json$/, ''))
    .sort()
}

async function readNamespace(locale: string, namespace: string) {
  const path = join(LOCALES_DIR, locale, `${namespace}.json`)
  const contents = await readFile(path, 'utf8')
  return JSON.parse(contents) as JsonObject
}

function flattenKeys(value: JsonValue, prefix = ''): string[] {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return [prefix]
  }

  return Object.entries(value).flatMap(([key, child]) =>
    flattenKeys(child, prefix ? `${prefix}.${key}` : key)
  )
}

const PLURAL_SUFFIX = /_(zero|one|two|few|many|other)$/

/**
 * Chinese uses only `_other` while English also needs `_one`, so compare the
 * base key sets and let each locale carry its own plural variants.
 */
function normalizePluralKeys(keys: string[]): string[] {
  return [...new Set(keys.map((key) => key.replace(PLURAL_SUFFIX, '')))].sort()
}

function diff(reference: string[], compared: string[]) {
  const comparedSet = new Set(compared)
  const referenceSet = new Set(reference)
  return {
    missing: reference.filter((key) => !comparedSet.has(key)),
    extra: compared.filter((key) => !referenceSet.has(key)),
  }
}

async function main() {
  const localeDirs = (await readdir(LOCALES_DIR, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()

  if (
    !localeDirs.includes(BASELINE_LOCALE) ||
    !localeDirs.includes(COMPARED_LOCALE)
  ) {
    throw new Error(
      `Expected locale directories ${BASELINE_LOCALE} and ${COMPARED_LOCALE}; found ${localeDirs.join(', ')}`
    )
  }

  const namespaces = await listNamespaces(BASELINE_LOCALE)
  const comparedNamespaces = await listNamespaces(COMPARED_LOCALE)
  const failures: string[] = []

  const namespaceDiff = diff(namespaces, comparedNamespaces)
  if (namespaceDiff.missing.length > 0) {
    failures.push(
      `missing namespace files in ${COMPARED_LOCALE}: ${namespaceDiff.missing.join(', ')}`
    )
  }
  if (namespaceDiff.extra.length > 0) {
    failures.push(
      `extra namespace files in ${COMPARED_LOCALE}: ${namespaceDiff.extra.join(', ')}`
    )
  }

  for (const namespace of namespaces) {
    const baseline = normalizePluralKeys(
      flattenKeys(await readNamespace(BASELINE_LOCALE, namespace))
    )
    const compared = normalizePluralKeys(
      flattenKeys(await readNamespace(COMPARED_LOCALE, namespace))
    )

    const { missing, extra } = diff(baseline, compared)
    if (missing.length > 0) {
      failures.push(
        `${namespace}: missing in ${COMPARED_LOCALE}: ${missing.join(', ')}`
      )
    }
    if (extra.length > 0) {
      failures.push(
        `${namespace}: extra in ${COMPARED_LOCALE}: ${extra.join(', ')}`
      )
    }
  }

  if (failures.length > 0) {
    throw new Error(`i18n key mismatch:\n${failures.join('\n')}`)
  }

  process.stdout.write(
    `i18n key check passed for ${namespaces.length} namespaces (${BASELINE_LOCALE} ↔ ${COMPARED_LOCALE})\n`
  )
}

await main()

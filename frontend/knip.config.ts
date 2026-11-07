import type { KnipConfig } from 'knip'

const config: KnipConfig = {
  // Retained for the users/roles/audit tickets and excluded from this
  // ticket's checks; the UI primitives are also excluded from coverage.
  ignore: [
    'src/components/ui/**',
    'src/components/data-table/**',
    'src/components/long-text.tsx',
    'src/components/select-dropdown.tsx',
  ],
  ignoreExportsUsedInFile: true,
  ignoreDependencies: ['@radix-ui/react-icons'],
}

export default config

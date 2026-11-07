import { defineConfig } from 'orval'

// The committed `backend/openapi.json` is the contract source of truth. This
// config turns it into the committed TanStack Query client under
// `src/api/generated/`; CI regenerates both and fails on any diff.
//
// `ajv` is a direct devDependency because orval's spec validator depends on
// ajv 8, while Bun's hoisting otherwise installs ajv 6 for eslint at the root.
export default defineConfig({
  api: {
    input: '../backend/openapi.json',
    output: {
      target: './src/api/generated/rs-admin.ts',
      schemas: './src/api/generated/model',
      client: 'react-query',
      httpClient: 'axios',
      clean: true,
      formatter: 'prettier',
      override: {
        mutator: {
          path: './src/api/mutator.ts',
          name: 'customInstance',
        },
      },
    },
  },
})

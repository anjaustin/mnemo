import { defineConfig } from 'tsup';

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    'ext/langchain': 'src/ext/langchain.ts',
    'ext/vercel-ai': 'src/ext/vercel-ai.ts',
  },
  format: ['cjs', 'esm'],
  dts: true,
  clean: true,
  sourcemap: true,
  splitting: false,
  treeshake: true,
  external: ['@langchain/core', 'ai'],
});

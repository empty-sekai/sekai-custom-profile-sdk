import { existsSync } from "node:fs";
import { registerHooks } from "node:module";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith(".") && specifier.endsWith(".js")) {
      const candidate = new URL(`${specifier.slice(0, -3)}.ts`, context.parentURL);
      if (existsSync(candidate)) {
        return { shortCircuit: true, url: candidate.href };
      }
    }
    if (specifier.startsWith(".") && !specifier.endsWith(".ts")) {
      const candidate = new URL(`${specifier}.ts`, context.parentURL);
      if (existsSync(candidate)) {
        return { shortCircuit: true, url: candidate.href };
      }
    }
    return nextResolve(specifier, context);
  },
});

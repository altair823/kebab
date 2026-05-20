// sample.tsx
import React from "react";
export function Hello({ name }: { name: string }) { return <span>{name}</span>; }
export const App = () => <Hello name="x" />;  // arrow fn assigned → glue

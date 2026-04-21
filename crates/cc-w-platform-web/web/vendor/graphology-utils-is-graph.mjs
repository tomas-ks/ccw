export default function isGraph(value) {
  return (
    value !== null &&
    typeof value === "object" &&
    typeof value.addUndirectedEdgeWithKey === "function" &&
    typeof value.dropNode === "function" &&
    typeof value.multi === "boolean"
  );
}

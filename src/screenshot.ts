export function getFullScreenshotPath(
  thumbnailPath: string,
): string | null {
  const match =
    thumbnailPath.match(
      /^(.*[\\/])capture-([^\\/]+)$/,
    );

  if (!match) {
    return null;
  }

  return (
    `${match[1]}` +
    `capture-full-${match[2]}`
  );
}

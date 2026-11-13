// Avatar image helper shared by the join dialog, the guild profile dialog and
// the device settings page.

// Resizes a picked image to a square 96 px JPEG data URL. Resolves null when
// the file cannot be read as an image.
export function resizeAvatar(file) {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => {
      const size = 96;
      const canvas = document.createElement('canvas');
      canvas.width = size;
      canvas.height = size;
      const side = Math.min(image.width, image.height);
      canvas.getContext('2d').drawImage(image, (image.width - side) / 2, (image.height - side) / 2, side, side, 0, 0, size, size);
      URL.revokeObjectURL(image.src);
      resolve(canvas.toDataURL('image/jpeg', 0.82));
    };
    image.onerror = () => resolve(null);
    image.src = URL.createObjectURL(file);
  });
}

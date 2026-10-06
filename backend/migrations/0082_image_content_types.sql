-- Covers and pictures uploaded as "application/octet-stream" (the Mac sends that for AVIF)
-- were served with it, and browsers showed no image. The name says what they are.
UPDATE media_objects
   SET content_type = CASE lower(substring(object_key from '\.([A-Za-z0-9]+)$'))
                          WHEN 'avif' THEN 'image/avif'
                          WHEN 'jpg'  THEN 'image/jpeg'
                          WHEN 'jpeg' THEN 'image/jpeg'
                          WHEN 'png'  THEN 'image/png'
                          WHEN 'webp' THEN 'image/webp'
                          WHEN 'gif'  THEN 'image/gif'
                          WHEN 'heic' THEN 'image/heic'
                      END
 WHERE content_type = 'application/octet-stream'
   AND lower(object_key) ~ '\.(avif|jpe?g|png|webp|gif|heic)$';

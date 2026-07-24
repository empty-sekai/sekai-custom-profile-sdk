#include <setjmp.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <jpeglib.h>

typedef struct {
  struct jpeg_error_mgr base;
  jmp_buf jump;
  unsigned char **output;
  char *message;
  size_t message_capacity;
} allium_jpeg_error;

static void allium_jpeg_error_exit(j_common_ptr common) {
  allium_jpeg_error *error = (allium_jpeg_error *)common->err;
  if (error->message != NULL && error->message_capacity > 0) {
    common->err->format_message(common, error->message);
    error->message[error->message_capacity - 1] = '\0';
  }
  if (error->output != NULL && *error->output != NULL) {
    free(*error->output);
    *error->output = NULL;
  }
  longjmp(error->jump, 1);
}

int allium_jpeg_encode_rgba(const uint8_t *rgba, uint32_t width,
                            uint32_t height, int quality,
                            unsigned char **output,
                            unsigned long *output_length, char *error_message,
                            size_t error_capacity) {
  if (rgba == NULL || width == 0 || height == 0 || output == NULL ||
      output_length == NULL || quality < 1 || quality > 100) {
    return 2;
  }

  struct jpeg_compress_struct encoder;
  allium_jpeg_error error;
  volatile int encoder_created = 0;
  memset(&encoder, 0, sizeof(encoder));
  memset(&error, 0, sizeof(error));
  *output = NULL;
  *output_length = 0;
  encoder.err = jpeg_std_error(&error.base);
  error.base.error_exit = allium_jpeg_error_exit;
  error.output = output;
  error.message = error_message;
  error.message_capacity = error_capacity;
  if (setjmp(error.jump) != 0) {
    if (encoder_created) {
      jpeg_destroy_compress(&encoder);
    }
    return 1;
  }

  jpeg_create_compress(&encoder);
  encoder_created = 1;
  jpeg_mem_dest(&encoder, output, output_length);
  encoder.image_width = width;
  encoder.image_height = height;
  encoder.input_components = 4;
  encoder.in_color_space = JCS_EXT_RGBA;
  jpeg_set_defaults(&encoder);
  jpeg_set_quality(&encoder, quality, TRUE);
  jpeg_start_compress(&encoder, TRUE);
  const size_t stride = (size_t)width * 4;
  while (encoder.next_scanline < encoder.image_height) {
    JSAMPROW row = (JSAMPROW)(rgba + (size_t)encoder.next_scanline * stride);
    jpeg_write_scanlines(&encoder, &row, 1);
  }
  jpeg_finish_compress(&encoder);
  jpeg_destroy_compress(&encoder);
  encoder_created = 0;
  return 0;
}

int allium_jpeg_encode_yuv420(const uint8_t *y_plane,
                              const uint8_t *cb_plane,
                              const uint8_t *cr_plane, uint32_t width,
                              uint32_t height, uint32_t y_stride,
                              uint32_t chroma_stride, int quality,
                              unsigned char **output,
                              unsigned long *output_length,
                              char *error_message, size_t error_capacity) {
  if (y_plane == NULL || cb_plane == NULL || cr_plane == NULL || width == 0 ||
      height == 0 || (width & 1) != 0 || (height & 1) != 0 ||
      y_stride < width || chroma_stride < width / 2 || output == NULL ||
      output_length == NULL || quality < 1 || quality > 100) {
    return 2;
  }

  struct jpeg_compress_struct encoder;
  allium_jpeg_error error;
  volatile int encoder_created = 0;
  memset(&encoder, 0, sizeof(encoder));
  memset(&error, 0, sizeof(error));
  *output = NULL;
  *output_length = 0;
  encoder.err = jpeg_std_error(&error.base);
  error.base.error_exit = allium_jpeg_error_exit;
  error.output = output;
  error.message = error_message;
  error.message_capacity = error_capacity;
  if (setjmp(error.jump) != 0) {
    if (encoder_created) {
      jpeg_destroy_compress(&encoder);
    }
    return 1;
  }

  jpeg_create_compress(&encoder);
  encoder_created = 1;
  jpeg_mem_dest(&encoder, output, output_length);
  encoder.image_width = width;
  encoder.image_height = height;
  encoder.input_components = 3;
  encoder.in_color_space = JCS_YCbCr;
  jpeg_set_defaults(&encoder);
  encoder.raw_data_in = TRUE;
  encoder.comp_info[0].h_samp_factor = 2;
  encoder.comp_info[0].v_samp_factor = 2;
  encoder.comp_info[1].h_samp_factor = 1;
  encoder.comp_info[1].v_samp_factor = 1;
  encoder.comp_info[2].h_samp_factor = 1;
  encoder.comp_info[2].v_samp_factor = 1;
  jpeg_set_quality(&encoder, quality, TRUE);
  jpeg_start_compress(&encoder, TRUE);

  JSAMPROW y_rows[16];
  JSAMPROW cb_rows[8];
  JSAMPROW cr_rows[8];
  JSAMPARRAY planes[3] = {y_rows, cb_rows, cr_rows};
  const uint32_t chroma_height = height / 2;
  while (encoder.next_scanline < encoder.image_height) {
    const uint32_t y_base = encoder.next_scanline;
    const uint32_t chroma_base = y_base / 2;
    for (uint32_t row = 0; row < 16; ++row) {
      const uint32_t source_row =
          y_base + row < height ? y_base + row : height - 1;
      y_rows[row] = (JSAMPROW)(y_plane + (size_t)source_row * y_stride);
    }
    for (uint32_t row = 0; row < 8; ++row) {
      const uint32_t source_row = chroma_base + row < chroma_height
                                      ? chroma_base + row
                                      : chroma_height - 1;
      cb_rows[row] =
          (JSAMPROW)(cb_plane + (size_t)source_row * chroma_stride);
      cr_rows[row] =
          (JSAMPROW)(cr_plane + (size_t)source_row * chroma_stride);
    }
    if (jpeg_write_raw_data(&encoder, planes, 16) != 16) {
      jpeg_destroy_compress(&encoder);
      if (*output != NULL) {
        free(*output);
        *output = NULL;
      }
      return 3;
    }
  }

  jpeg_finish_compress(&encoder);
  jpeg_destroy_compress(&encoder);
  encoder_created = 0;
  return 0;
}

void allium_jpeg_free(void *output) { free(output); }

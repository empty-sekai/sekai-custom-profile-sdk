#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <x264.h>

typedef struct {
    x264_t *encoder;
    x264_picture_t picture;
    uint8_t *output;
    size_t output_capacity;
    int width;
    int height;
} allium_x264_encoder;

static int copy_nals(allium_x264_encoder *state,
                     x264_nal_t *nals,
                     int nal_count,
                     const uint8_t **output,
                     size_t *output_size) {
    size_t required = 0;
    for (int index = 0; index < nal_count; ++index) {
        required += (size_t)nals[index].i_payload;
    }
    if (required > state->output_capacity) {
        size_t capacity = state->output_capacity ? state->output_capacity : 65536;
        while (capacity < required) {
            capacity *= 2;
        }
        uint8_t *replacement = (uint8_t *)realloc(state->output, capacity);
        if (!replacement) {
            return -1;
        }
        state->output = replacement;
        state->output_capacity = capacity;
    }
    size_t offset = 0;
    for (int index = 0; index < nal_count; ++index) {
        memcpy(state->output + offset, nals[index].p_payload, (size_t)nals[index].i_payload);
        offset += (size_t)nals[index].i_payload;
    }
    *output = state->output;
    *output_size = offset;
    return 0;
}

allium_x264_encoder *allium_x264_create(int width, int height, int fps, int qp) {
    if (width <= 0 || height <= 0 || fps <= 0 || qp < 0 || qp > 51) {
        return NULL;
    }
    allium_x264_encoder *state = (allium_x264_encoder *)calloc(1, sizeof(*state));
    if (!state) {
        return NULL;
    }
    x264_param_t param;
    if (x264_param_default_preset(&param, "ultrafast", "zerolatency") < 0) {
        free(state);
        return NULL;
    }
    param.i_width = width;
    param.i_height = height;
    param.i_csp = X264_CSP_I420;
    param.i_fps_num = fps;
    param.i_fps_den = 1;
    param.i_timebase_num = 1;
    param.i_timebase_den = fps;
    param.b_vfr_input = 0;
    param.i_threads = 1;
    param.b_sliced_threads = 0;
    param.i_sync_lookahead = 0;
    param.rc.i_lookahead = 0;
    param.i_bframe = 0;
    param.i_keyint_max = fps * 8;
    param.i_keyint_min = fps * 8;
    param.i_scenecut_threshold = 0;
    param.i_frame_reference = 1;
    param.b_cabac = 0;
    param.b_deblocking_filter = 0;
    param.analyse.i_me_method = X264_ME_DIA;
    param.analyse.i_me_range = 4;
    param.analyse.i_subpel_refine = 0;
    param.analyse.i_trellis = 0;
    param.analyse.b_transform_8x8 = 0;
    param.analyse.b_chroma_me = 0;
    param.analyse.b_psy = 0;
    param.analyse.b_mb_info = 1;
    param.analyse.i_weighted_pred = X264_WEIGHTP_NONE;
    param.analyse.b_weighted_bipred = 0;
    param.rc.i_aq_mode = X264_AQ_NONE;
    param.rc.b_mb_tree = 0;
    param.rc.i_rc_method = X264_RC_CQP;
    param.rc.i_qp_constant = qp;
    param.i_log_level = X264_LOG_NONE;
    param.b_repeat_headers = 1;
    param.b_annexb = 1;
    if (x264_param_apply_profile(&param, "baseline") < 0) {
        free(state);
        return NULL;
    }
    state->encoder = x264_encoder_open(&param);
    if (!state->encoder) {
        free(state);
        return NULL;
    }
    x264_picture_init(&state->picture);
    state->picture.img.i_csp = X264_CSP_I420;
    state->picture.img.i_plane = 3;
    state->picture.img.i_stride[0] = width;
    state->picture.img.i_stride[1] = width / 2;
    state->picture.img.i_stride[2] = width / 2;
    state->width = width;
    state->height = height;
    return state;
}

int allium_x264_encode(allium_x264_encoder *state,
                       const uint8_t *yuv420p,
                       const uint8_t *mb_info,
                       int64_t pts,
                       const uint8_t **output,
                       size_t *output_size) {
    if (!state || !yuv420p || !mb_info || !output || !output_size) {
        return -1;
    }
    const size_t y_bytes = (size_t)state->width * (size_t)state->height;
    const size_t chroma_bytes = y_bytes / 4;
    state->picture.img.plane[0] = (uint8_t *)yuv420p;
    state->picture.img.plane[1] = (uint8_t *)yuv420p + y_bytes;
    state->picture.img.plane[2] = (uint8_t *)yuv420p + y_bytes + chroma_bytes;
    state->picture.prop.mb_info = (uint8_t *)mb_info;
    state->picture.i_pts = pts;
    x264_nal_t *nals = NULL;
    int nal_count = 0;
    x264_picture_t output_picture;
    int encoded = x264_encoder_encode(
        state->encoder, &nals, &nal_count, &state->picture, &output_picture);
    if (encoded < 0 || copy_nals(state, nals, nal_count, output, output_size) < 0) {
        return -1;
    }
    return encoded;
}

int allium_x264_flush(allium_x264_encoder *state,
                      const uint8_t **output,
                      size_t *output_size) {
    if (!state || !output || !output_size) {
        return -1;
    }
    x264_nal_t *nals = NULL;
    int nal_count = 0;
    x264_picture_t output_picture;
    int encoded = x264_encoder_encode(state->encoder, &nals, &nal_count, NULL, &output_picture);
    if (encoded < 0 || copy_nals(state, nals, nal_count, output, output_size) < 0) {
        return -1;
    }
    return encoded;
}

int allium_x264_delayed_frames(const allium_x264_encoder *state) {
    return state ? x264_encoder_delayed_frames(state->encoder) : 0;
}

void allium_x264_destroy(allium_x264_encoder *state) {
    if (!state) {
        return;
    }
    if (state->encoder) {
        x264_encoder_close(state->encoder);
    }
    free(state->output);
    free(state);
}

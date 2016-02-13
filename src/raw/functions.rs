use std::ptr;
use std::mem;
use std::io::Read;
use std::ffi::CString;
use std::iter;
use libc::{size_t, c_void};
use num::{FromPrimitive};

use cl_h::{self, cl_platform_id, cl_device_id, cl_device_type, cl_device_info, cl_context,
    cl_context_info, cl_platform_info, cl_image_format, cl_image_desc, cl_mem_flags, cl_kernel,
    cl_program, cl_program_build_info, cl_command_queue, cl_mem, cl_event, ClStatus};

use super::super::{DEFAULT_DEVICE_TYPE, DEVICES_MAX, Error as OclError, Result as OclResult};



//=============================================================================
//============================ SUPPORT FUNCTIONS ==============================
//=============================================================================

fn errcode_string(errcode: i32) -> String {
    match ClStatus::from_i32(errcode) {
        Some(cls) => format!("{:?}", cls),
        None => format!("[Unknown Error Code: {}]", errcode as i64),
    }
}

/// Evaluates `errcode` and returns an `Err` if it is not 0.
pub fn errcode_try(message: &str, errcode: i32) -> OclResult<()> {
    if errcode != cl_h::ClStatus::CL_SUCCESS as i32 {
        OclError::err(format!("\n\nOPENCL ERROR: {} failed with code: {}\n\n", 
            message, errcode_string(errcode)))
    } else {
        Ok(())
    }
}

/// Evaluates `errcode` and panics with a failure message if it is not 0.
pub fn errcode_assert(message: &str, errcode: i32) {
    errcode_try(message, errcode).unwrap();
}

/// Maps options of slices to pointers and a length.
pub fn resolve_queue_opts(wait_list: Option<&[cl_event]>, dest_event: Option<&mut cl_event>)
        -> OclResult<(u32, *const cl_event, *mut cl_event)>
{
    // If the wait list is empty or if its containing option is none, map to (0, null),
    // otherwise map to the length and pointer:
    let (wait_list_len, wait_list_ptr): (u32, *const cl_event) = match wait_list {
        Some(wl) => {
            if wl.len() > 0 {
                (wl.len() as u32, wl.as_ptr())
            } else {
                (0, ptr::null())
            }
        },
        None => (0, ptr::null()),
    };

    // If the new event 
    let new_event_ptr: *mut cl_event = match dest_event {
        Some(de) => {
            de
        },
        None => ptr::null_mut(),
    };

    Ok((wait_list_len, wait_list_ptr, new_event_ptr))
}

pub fn resolve_work_dims(work_dims: &Option<[usize; 3]>) -> *const size_t {
    match work_dims {
        &Some(ref w) => w as *const [usize; 3] as *const size_t,
        &None => 0 as *const size_t,
    }
}


//=============================================================================
//============================== OCL FUNCTIONS ================================
//=============================================================================

/// Returns a list of available platforms by id.
pub fn get_platform_ids() -> Vec<cl_platform_id> {
    let mut num_platforms = 0 as u32;
    
    // Get a count of available platforms:
    let mut errcode: i32 = unsafe { 
        cl_h::clGetPlatformIDs(0, ptr::null_mut(), &mut num_platforms) 
    };
    errcode_assert("clGetPlatformIDs()", errcode);

    // Create a vec with the appropriate size:
    let mut platforms: Vec<cl_platform_id> = iter::repeat(0 as cl_platform_id)
        .take(num_platforms as usize).collect();

    errcode = unsafe {
        cl_h::clGetPlatformIDs(num_platforms, platforms.as_mut_ptr(), ptr::null_mut())
    };
    errcode_assert("clGetPlatformIDs()", errcode);
    
    platforms
}

/// Returns a list of available devices for a particular platform by id.
///
/// # Panics
///
///  -errcode_assert(): [FIXME]: Explaination needed (possibly at crate level?)
pub fn get_device_ids(
        platform: cl_platform_id, 
        device_types_opt: Option<cl_device_type>)
        -> Vec<cl_device_id> 
{
    let device_type = device_types_opt.unwrap_or(DEFAULT_DEVICE_TYPE);
    
    let mut devices_available: u32 = 0;
    let mut devices_array: [cl_device_id; DEVICES_MAX as usize] = [0 as cl_device_id; DEVICES_MAX as usize];

    let errcode = unsafe { cl_h::clGetDeviceIDs(
        platform, 
        device_type, 
        DEVICES_MAX, 
        devices_array.as_mut_ptr(), 
        &mut devices_available
    ) };

    errcode_assert("clGetDeviceIDs()", errcode);

    let mut device_ids: Vec<cl_device_id> = Vec::with_capacity(devices_available as usize);

    for i in 0..devices_available as usize {
        device_ids.push(devices_array[i]);
    }

    device_ids
}

pub fn create_context(device_ids: &Vec<cl_device_id>) -> cl_context {
    let mut errcode: i32 = 0;

    unsafe {
        let context: cl_context = cl_h::clCreateContext(
            ptr::null(), 
            device_ids.len() as u32, 
            device_ids.as_ptr(),
            mem::transmute(ptr::null::<fn()>()), 
            ptr::null_mut(), 
            &mut errcode);
        errcode_assert("clCreateContext()", errcode);
        context
    }
}

pub fn create_build_program(
            kern_strings: Vec<CString>,
            cmplr_opts: CString,
            context: cl_context, 
            device_ids: &Vec<cl_device_id>)
            -> OclResult<cl_program>
{
    // Verify that the context is valid:
    try!(verify_context(context));

    // Lengths (not including \0 terminator) of each string:
    let ks_lens: Vec<usize> = kern_strings.iter().map(|cs| cs.as_bytes().len()).collect();  
    // Pointers to each string:
    let kern_string_ptrs: Vec<*const i8> = kern_strings.iter().map(|cs| cs.as_ptr()).collect();

    let mut errcode = 0i32;
    
    let program: cl_program = unsafe { cl_h::clCreateProgramWithSource(
                context, 
                kern_string_ptrs.len() as u32,
                kern_string_ptrs.as_ptr() as *const *const i8,
                ks_lens.as_ptr() as *const usize,
                &mut errcode,
    )};
    errcode_assert("clCreateProgramWithSource()", errcode);

    errcode = unsafe { cl_h::clBuildProgram(
                program,
                device_ids.len() as u32,
                device_ids.as_ptr(), 
                cmplr_opts.as_ptr() as *const i8,
                mem::transmute(ptr::null::<fn()>()), 
                ptr::null_mut(),
    )};  

    if errcode < 0 {
        program_build_err(program, device_ids).map(|_| program)         
    } else {
        try!(errcode_try("clBuildProgram()", errcode));
        Ok(program) 
    }
}


pub fn create_kernel(
            program: cl_program, 
            name: &str)
            -> OclResult<cl_kernel>
{
    let mut err: i32 = 0;

    let kernel_obj = unsafe {
        cl_h::clCreateKernel(
            program, 
            try!(CString::new(name.as_bytes())).as_ptr(), 
            &mut err,
        )
    };    
    let err_pre = format!("clCreateKernel('{}'):", &name);
    try!(errcode_try(&err_pre, err));
    Ok(kernel_obj)
}


    // // Modifies a kernel argument.
    // fn set_kernel_arg(&mut self, arg_index: u32, arg_size: libc::size_t, 
    //             arg_value: *const libc::c_void) 
    // {
    //     unsafe {
    //         let err = cl_h::clSetKernelArg(
    //                     self.kernel_obj, 
    //                     arg_index,
    //                     arg_size, 
    //                     arg_value,
    //         );

    //         let err_pre = format!("ocl::Kernel::set_kernel_arg('{}'):", &self.name);
    //         raw::errcode_assert(&err_pre, err);
    //     }
    // } 


pub fn create_command_queue(
            context: cl_context, 
            device: cl_device_id)
            -> OclResult<cl_command_queue>
{
    // Verify that the context is valid:
    try!(verify_context(context));

    let mut errcode: i32 = 0;

    unsafe {
        let cq: cl_command_queue = cl_h::clCreateCommandQueue(
                    context, 
                    device, 
                    cl_h::CL_QUEUE_PROFILING_ENABLE, 
                    &mut errcode
        );

        errcode_assert("clCreateCommandQueue()", errcode);
        Ok(cq)
    }
}

pub fn create_buffer<T>(
            context: cl_context,
            flags: cl_mem_flags,
            len: usize,
            data: Option<&[T]>)
            -> OclResult<cl_mem>
{
    // Verify that the context is valid:
    try!(verify_context(context));

    let mut errcode: i32 = 0;

    let host_ptr = match data {
        Some(d) => {
            assert!(d.len() == len, "ocl::create_buffer(): Data length mismatch.");
            d.as_ptr() as *mut c_void
        },
        None => ptr::null_mut(),
    };

    let buf = unsafe { cl_h::clCreateBuffer(
            context, 
            flags,
            len * mem::size_of::<T>(),
            host_ptr, 
            &mut errcode,
    )};
    errcode_assert("create_buffer", errcode);

    Ok(buf)
}


// [WORK IN PROGRESS]
pub fn create_image<T>(
            context: cl_context,
            flags: cl_mem_flags,
            format: &cl_image_format,
            desc: &cl_image_desc,
            data: Option<&[T]>)
            -> OclResult<cl_mem>
{
    // Verify that the context is valid:
    try!(verify_context(context));

    let mut errcode: i32 = 0;
    
    let data_ptr = match data {
        Some(d) => {
            // [FIXME]: CALCULATE CORRECT IMAGE SIZE AND COMPARE
            // assert!(d.len() == len, "ocl::create_image(): Data length mismatch.");
            d.as_ptr() as *mut c_void
        },
        None => ptr::null_mut(),
    };

    let image_obj = unsafe { cl_h::clCreateImage(
            context,
            flags,
            format as *const cl_image_format,
            desc as *const cl_image_desc,
            data_ptr,
            &mut errcode as *mut i32)
    }; 
    errcode_assert("create_image", errcode);

    assert!(!image_obj.is_null());

    Ok(image_obj)
}

pub fn enqueue_write_buffer<T>(
            command_queue: cl_command_queue,
            buffer: cl_mem, 
            block: bool,
            data: &[T],
            offset: usize,
            wait_list: Option<&[cl_event]>, 
            dest_event: Option<&mut cl_event>)
{
    let (wait_list_len, wait_list_ptr, new_event_ptr) 
        = resolve_queue_opts(wait_list, dest_event).expect("[FIXME]: enqueue_write_buffer()");

    unsafe {
        let errcode = cl_h::clEnqueueWriteBuffer(
                    command_queue,
                    buffer,
                    block as u32,
                    offset,
                    (data.len() * mem::size_of::<T>()) as size_t,
                    data.as_ptr() as *const c_void,
                    wait_list_len,
                    wait_list_ptr,
                    new_event_ptr,
        );

        errcode_assert("clEnqueueWriteBuffer()", errcode);
    }
}

pub fn enqueue_read_buffer<T>(
            command_queue: cl_command_queue,
            buffer: cl_mem, 
            block: bool,
            data: &[T],
            offset: usize,
            wait_list: Option<&[cl_event]>, 
            dest_event: Option<&mut cl_event>)
{
    let (wait_list_len, wait_list_ptr, new_event_ptr) = 
        resolve_queue_opts(wait_list, dest_event).expect("[FIXME]: enqueue_read_buffer()");

    unsafe {
        let errcode = cl_h::clEnqueueReadBuffer(
                    command_queue, 
                    buffer, 
                    block as u32, 
                    offset, 
                    (data.len() * mem::size_of::<T>()) as size_t, 
                    data.as_ptr() as *mut c_void, 
                    wait_list_len,
                    wait_list_ptr,
                    new_event_ptr,
        );

        errcode_assert("clEnqueueReadBuffer()", errcode);
    }
}


pub fn enqueue_kernel(
            command_queue: cl_command_queue,
            kernel: cl_kernel,
            work_dims: u32,
            global_work_offset: Option<[usize; 3]>,
            global_work_dims: [usize; 3],
            local_work_dims: Option<[usize; 3]>,
            wait_list: Option<&[cl_event]>, 
            dest_event: Option<&mut cl_event>,
            kernel_name: Option<&str>)
{
    let (wait_list_len, wait_list_ptr, new_event_ptr) = 
        resolve_queue_opts(wait_list, dest_event).expect("[FIXME]: enqueue_kernel()");
    let gwo = resolve_work_dims(&global_work_offset);
    let gws = &global_work_dims as *const size_t;
    let lws = resolve_work_dims(&local_work_dims);

    unsafe {
        let errcode = cl_h::clEnqueueNDRangeKernel(
            command_queue,
            kernel,
            work_dims,
            gwo,
            gws,
            lws,
            wait_list_len,
            wait_list_ptr,
            new_event_ptr,
        );

        let errcode_pre = format!("ocl::Kernel::enqueue()[{}]:", kernel_name.unwrap_or(""));
        errcode_assert(&errcode_pre, errcode);
    }
}


/// [UNTESTED][UNUSED]
#[allow(dead_code)]
pub fn enqueue_copy_buffer(
                command_queue: cl_command_queue,
                src_buffer: cl_mem,
                dst_buffer: cl_mem,
                src_offset: usize,
                dst_offset: usize,
                len_copy_bytes: usize)
{
    unsafe {
        let errcode = cl_h::clEnqueueCopyBuffer(
            command_queue,
            src_buffer,
            dst_buffer,
            src_offset,
            dst_offset,
            len_copy_bytes as usize,
            0,
            ptr::null(),
            ptr::null_mut(),
        );
        errcode_assert("clEnqueueCopyBuffer()", errcode);
    }
}

pub fn get_max_work_group_size(device: cl_device_id) -> usize {
    let mut max_work_group_size: usize = 0;

    let errcode = unsafe { 
        cl_h::clGetDeviceInfo(
            device,
            cl_h::CL_DEVICE_MAX_WORK_GROUP_SIZE,
            mem::size_of::<usize>() as usize,
            &mut max_work_group_size as *mut _ as *mut c_void,
            ptr::null_mut(),
        ) 
    }; 

    errcode_assert("clGetDeviceInfo", errcode);

    max_work_group_size
}

pub fn finish(command_queue: cl_command_queue) {
    unsafe { 
        let errcode = cl_h::clFinish(command_queue);
        errcode_assert("clFinish()", errcode);
    }
}

pub fn wait_for_events(count: u32, event_list: &[cl_event]) {
    let errcode = unsafe {
        cl_h::clWaitForEvents(count, event_list.as_ptr())
    };

    errcode_assert("clWaitForEvents", errcode);
}


#[allow(dead_code)]
pub fn wait_for_event(event: cl_event) {
    let event_array: [cl_event; 1] = [event];

    let errcode = unsafe {
        cl_h::clWaitForEvents(1, event_array.as_ptr())
    };

    errcode_assert("clWaitForEvents", errcode);
}

pub fn get_event_status(event: cl_event) -> i32 {
    let mut status: i32 = 0;

    let errcode = unsafe { 
        cl_h::clGetEventInfo(
            event,
            cl_h::CL_EVENT_COMMAND_EXECUTION_STATUS,
            mem::size_of::<i32>(),
            &mut status as *mut _ as *mut c_void,
            ptr::null_mut(),
        )
    };

    errcode_assert("clGetEventInfo", errcode);

    status
}

pub unsafe fn set_event_callback(
            event: cl_event, 
            callback_trigger: i32, 
            callback_receiver: extern fn (cl_event, i32, *mut c_void),
            user_data: *mut c_void)
{
    let errcode = cl_h::clSetEventCallback(event, callback_trigger, callback_receiver, user_data);

    errcode_assert("clSetEventCallback", errcode);
}

pub fn release_event(event: cl_event) {
    let errcode = unsafe {
        cl_h::clReleaseEvent(event)
    };

    errcode_assert("clReleaseEvent", errcode);
}

pub fn release_mem_object(obj: cl_mem) {
    unsafe {
        cl_h::clReleaseMemObject(obj);
    }
}

/// TODO: Evaluate usefulness
#[allow(dead_code)]
pub fn platform_info(platform: cl_platform_id) {
    let mut size = 0 as size_t;

    unsafe {
        let name = cl_h::CL_PLATFORM_NAME as cl_platform_info;
        let mut errcode = cl_h::clGetPlatformInfo(
                    platform,
                    name,
                    0,
                    ptr::null_mut(),
                    &mut size,
        );
        errcode_assert("clGetPlatformInfo(size)", errcode);
        
        let mut param_value: Vec<u8> = iter::repeat(32u8).take(size as usize).collect();
        errcode = cl_h::clGetPlatformInfo(
                    platform,
                    name,
                    size,
                    param_value.as_mut_ptr() as *mut c_void,
                    ptr::null_mut(),
        );
        errcode_assert("clGetPlatformInfo()", errcode);
        println!("*** Platform Name ({}): {}", name, String::from_utf8(param_value).unwrap());
    }
}

/// If the program pointed to by `cl_program` for any of the devices listed in `device_ids` has a build log of any length, it will be returned as an errcode result.
///
pub fn program_build_err(program: cl_program, device_ids: &Vec<cl_device_id>) -> OclResult<()> 
{
    let mut size = 0 as size_t;

    for &device_id in device_ids.iter() {
        unsafe {
            let name = cl_h::CL_PROGRAM_BUILD_LOG as cl_program_build_info;

            let mut errcode = cl_h::clGetProgramBuildInfo(
                program,
                device_id,
                name,
                0,
                ptr::null_mut(),
                &mut size,
            );
            errcode_assert("clGetProgramBuildInfo(size)", errcode);

            let mut pbi: Vec<u8> = iter::repeat(32u8).take(size as usize).collect();

            errcode = cl_h::clGetProgramBuildInfo(
                program,
                device_id,
                name,
                size,
                pbi.as_mut_ptr() as *mut c_void,
                ptr::null_mut(),
            );
            errcode_assert("clGetProgramBuildInfo()", errcode);

            if size > 1 {
                let pbi_nonull = try!(String::from_utf8(pbi).map_err(|e| e.to_string()));
                let pbi_errcode_string = format!(
                    "\n\n\
                    ###################### OPENCL PROGRAM BUILD DEBUG OUTPUT ######################\
                    \n\n{}\n\
                    ###############################################################################\
                    \n\n",
                    pbi_nonull);

                return OclError::err(pbi_errcode_string);
            }
        }
    }

    Ok(())
}

/// Returns a string containing requested information.
///
/// Currently lazily assumes everything is a char[] and converts to a String. 
/// Non-string info types need to be manually reconstructed from that. Yes this
/// is retarded.
///
/// [TODO (low priority)]: Needs to eventually be made more flexible and should return 
/// an enum with a variant corresponding to the type of info requested. Could 
/// alternatively return a generic type and just blindly cast to it.
#[allow(dead_code, unused_variables)] 
pub fn device_info(device: cl_device_id, info_type: cl_device_info) -> String {
    let mut info_value_size: usize = 0;

    let errcode = unsafe { 
        cl_h::clGetDeviceInfo(
            device,
            cl_h::CL_DEVICE_MAX_WORK_GROUP_SIZE,
            mem::size_of::<usize>() as usize,
            0 as *mut c_void,
            &mut info_value_size as *mut usize,
        ) 
    }; 

    errcode_assert("clGetDeviceInfo", errcode);

    String::new()
}

/// Returns context information.
///
/// [SDK Reference](https://www.khronos.org/registry/cl/sdk/1.2/docs/man/xhtml/clGetContextInfo.html)
///
/// # Errors
///
/// Returns an error result for all the reasons listed in the SDK in addition 
/// to an additional error when called with `CL_CONTEXT_DEVICES` as described
/// in in the `verify_context()` documentation below.
///
/// TODO: Finish wiring up full functionality. Return a 'ContextInfo' enum result.
pub fn context_info(context: cl_context, info_kind: cl_context_info) 
        -> OclResult<()>
{
    let mut result_size = 0;

    // let info_kind: cl_context_info = cl_h::CL_CONTEXT_PROPERTIES;
    let errcode = unsafe { cl_h::clGetContextInfo(   
        context,
        info_kind,
        0,
        0 as *mut c_void,
        &mut result_size as *mut usize,
    )};
    try!(errcode_try("clGetContextInfo", errcode));
    // println!("context_info(): errcode: {}, result_size: {}", errcode, result_size);

    let err_if_zero_result_size = info_kind == cl_h::CL_CONTEXT_DEVICES;

    if result_size > 10000 || (result_size == 0 && err_if_zero_result_size) {
        return OclError::err("\n\nocl::raw::context_info(): Possible invalid context detected. \n\
            Context info result size is either '> 10k bytes' or '== 0'. Almost certainly an \n\
            invalid context object. If not, please file an issue at: \n\
            https://github.com/cogciprocate/ocl/issues.\n\n");
    }

    let mut result: Vec<u8> = iter::repeat(0).take(result_size).collect();

    let errcode = unsafe { cl_h::clGetContextInfo(   
        context,
        info_kind,
        result_size,
        result.as_mut_ptr() as *mut c_void,
        0 as *mut usize,
    )};
    try!(errcode_try("clGetContextInfo", errcode));
    // println!("context_info(): errcode: {}, result: {:?}", errcode, result);

    Ok(())
}

/// Verifies that the `context` is in fact a context object pointer.
///
/// # Assumptions
///
/// Some (most?) OpenCL implementations do not correctly error if non-context pointers are passed. This function relies on the fact that passing the `CL_CONTEXT_DEVICES` as the `param_name` to `clGetContextInfo` will (on my AMD implementation at least) often return a huge result size if `context` is not actually a `cl_context` pointer due to the fact that it's reading from some random memory location on non-context objects. Also checks for zero because a context must have at least one device (true?).
pub fn verify_context(context: cl_context) -> OclResult<()> {
    // context_info(context, cl_h::CL_CONTEXT_REFERENCE_COUNT)
    context_info(context, cl_h::CL_CONTEXT_DEVICES)
}
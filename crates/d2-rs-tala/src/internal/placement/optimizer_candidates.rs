// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// []int candidate headers: copies share backing, length/capacity remain local.
#[derive(Clone,Debug,Default)]
struct CandidateSlice {
    backing:Option<std::sync::Arc<crate::internal::go_cell::GoCell<Box<[usize]>>>>,
    len:usize,
    cap:usize,
}
impl CandidateSlice {
    fn len(&self)->usize{self.len}
    fn clear(&mut self){self.len=0;}
    fn push(&mut self,value:usize){
        if self.len==self.cap{
            let capacity=super::container_slices::go127_slice_growth_capacity(self.cap,self.len+1,8,false).expect("bounded spatial candidate capacity");
            let mut slots=vec![0;capacity].into_boxed_slice();let old=self.values();slots[..self.len].copy_from_slice(&old);
            self.backing=Some(std::sync::Arc::new(crate::internal::go_cell::GoCell::new(slots)));self.cap=capacity;
        }
        self.backing.as_ref().expect("positive candidate capacity").write().expect("candidate backing lock")[self.len]=value;self.len+=1;
    }
    fn values(&self)->Vec<usize>{self.backing.as_ref().map_or_else(Vec::new,|b|b.read().expect("candidate backing lock")[..self.len].to_vec())}
    fn sort_unstable(&mut self){if let Some(backing)=&self.backing{backing.write().expect("candidate backing lock")[..self.len].sort_unstable();}}
}
struct CandidateIter{header:CandidateSlice,index:usize}
impl Iterator for CandidateIter{type Item=usize;fn next(&mut self)->Option<usize>{if self.index>=self.header.len{return None;}let value=self.header.backing.as_ref().expect("visible candidate backing").read().expect("candidate backing lock")[self.index];self.index+=1;Some(value)}}
impl IntoIterator for CandidateSlice{type Item=usize;type IntoIter=CandidateIter;fn into_iter(self)->Self::IntoIter{CandidateIter{header:self,index:0}}}
